//! Abstract syntax tree, elaboration, evaluator, and primitives — the
//! language core of the SATySFi port.

pub mod ast;
pub(crate) mod compile;
pub mod crossref;
pub mod elaborate;
pub mod eval;
pub mod exhaustive;
pub mod hyphenation;
pub mod prim_types;
pub mod primitives;
pub mod quoted;
pub mod symbol;
pub mod typecheck;
pub mod types;
pub mod unify;
pub mod v1;
pub mod value;

use crossref::{CrossRefs, Verdict};
use rustyfi_backend::{
    place_block_at, placed_line_extent, DecoId, FontMetrics, GraphicsElem, Length, PureHorzBox,
    VertBox,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use value::{DocumentValue, Value};

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(transparent)]
    Parse(#[from] rustyfi_syntax::ParseFileError),
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
    /// A `V0_0` dependency spliced into a `V0_1` program referenced `name`, a
    /// builtin primitive/type that is version-forked (bound, or shaped,
    /// differently between `V0_0` and `V0_1` — see
    /// `typecheck::forked_type_names`). The
    /// merged program's single `base_env_with_version(V0_1)` can only bind
    /// ONE closure per name, so accepting this would silently mis-resolve
    /// `name` to the WRONG version's primitive.
    ///
    /// The trailing `— {}` is `v1::xver_adapt::forked_note`, keyed on
    /// `name`: WHY this particular name cannot cross — a missing bridge
    /// feature (a wrapper could be written), or a REPRESENTATION fork
    /// (`page`, `font`) where the generations disagree about what the
    /// runtime value IS and no amount of bridge work helps.
    #[error(
        "cross-version import ({slice}): dependency {dep} references `{name}`, a \
         version-forked builtin — {}",
        v1::xver_adapt::forked_note(.name)
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
    let file = rustyfi_syntax::parse_file(src)?;
    compile_document_cst(&file, metrics)
}

/// Compile an already-parsed (possibly loader-merged) file. The multi-file
/// loader concatenates library preludes into one synthetic `cst::File` and
/// enters here.
///
/// Thin wrapper over [`compile_document_cst_with_trials`] that drops the
/// trial count — the stable entry point for the CLI (`main.rs`).
pub fn compile_document_cst(
    file: &rustyfi_syntax::cst::File,
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    compile_document_cst_with_trials(file, metrics).map(|(doc, _trials)| doc)
}

/// Same as [`compile_document_cst`], but also returns how many fixpoint
/// trials it took (& the fixpoint) — exposed for tests that must confirm the
/// fixpoint actually iterated, not just that it produced the right answer on
/// a lucky first pass.
pub fn compile_document_cst_with_trials(
    file: &rustyfi_syntax::cst::File,
    metrics: &dyn FontMetrics,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    compile_document_cst_with_aux(file, metrics, &mut crossref::AuxTable::new())
}

/// [`compile_document_cst_with_trials`] threading an AUXILIARY cross-reference table: `aux` seeds the
/// fixpoint from a previous run and is overwritten with the final table.
/// Seeding only affects how fast the fixpoint converges — see
/// [`crossref::CrossRefs::seeded`] and [`crossref::CrossRefs::seed_unvalidated`],
/// which together guarantee the output is the same as a cold run's.
pub fn compile_document_cst_with_aux(
    file: &rustyfi_syntax::cst::File,
    metrics: &dyn FontMetrics,
    aux: &mut crossref::AuxTable,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    compile_document_cst_with_stages(file, metrics, aux, &std::collections::HashMap::new())
}

/// The stage a file's `@stage:` header declares, if any.
///
/// The loader merges every library's prelude into one file and drops the
/// headers, so each caller that merges has to read this off first and record
/// which entries it covers -- see [`compile_document_cst_with_stages`].
pub fn declared_stage(file: &rustyfi_syntax::cst::File) -> Option<types::Stage> {
    use rustyfi_syntax::token::Token;
    file.headers.iter().find_map(|h| match h {
        rustyfi_syntax::cst::Header::Stage(st) => match st.tok {
            Token::HeaderPersistent0 => Some(types::Stage::Persistent0),
            Token::HeaderStage0 => Some(types::Stage::Stage0),
            Token::HeaderStage1 => Some(types::Stage::Stage1),
            _ => None,
        },
        _ => None,
    })
}

/// Record `file`'s declared stage against the prelude slots `start..end` its
/// bindings just landed in, when that stage is not the default.
fn note_stage(
    stages: &mut std::collections::HashMap<usize, types::Stage>,
    file: &rustyfi_syntax::cst::File,
    start: usize,
    end: usize,
) {
    if let Some(stage) = declared_stage(file).filter(|s| *s != types::Stage::default()) {
        stages.extend((start..end).map(|i| (i, stage)));
    }
}

/// Splice compiler-generated cross-version glue at the END of `prelude` and
/// tag it with `stage`, the DECLARED stage of the dependency whose bindings
/// the glue names.
///
/// The stage is load-bearing, not bookkeeping. `Stage::can_reference` is not
/// symmetric: a `@stage: persistent` binding may not read a default-stage one.
/// Every generated wrapper/shadow here re-applies a dependency's own export by
/// name, so splicing it at the default stage silently makes it unreadable from
/// the very consumers the forward deco/paren wrapper and its view-scheduling
/// exist to serve — the failure surfaces as
/// `invalid occurrence of variable .. as to stage`, which reads like a user
/// error in a document that mentions neither binding. A FIXED stage would be
/// as wrong for a `@stage: 0`/default dependency as the default is for a
/// persistent one (cf. `unite_helper_prelude`'s explicit `Persistent0`).
fn splice_staged(
    prelude: &mut Vec<rustyfi_syntax::cst::TopBinding>,
    stages: &mut std::collections::HashMap<usize, types::Stage>,
    stage: Option<types::Stage>,
    bindings: Vec<rustyfi_syntax::cst::TopBinding>,
) {
    let start = prelude.len();
    prelude.extend(bindings);
    if let Some(st) = stage {
        stages.extend((start..prelude.len()).map(|i| (i, st)));
    }
}

/// One [`v1::xver_adapt::deco_upgrade_prelude`] call per DISTINCT declared
/// stage among `exports`, each spliced at that stage.
///
/// Grouping rather than one flat call is what keeps [`splice_staged`]'s
/// argument true when a program crosses exports from dependencies that
/// declared DIFFERENT `@stage:` headers: the glue for each is emitted in its
/// own contiguous, correctly-tagged run. With one stage (every real program so
/// far) it is exactly one call, in `exports` order.
fn splice_upgrade_glue(
    prelude: &mut Vec<rustyfi_syntax::cst::TopBinding>,
    stages: &mut std::collections::HashMap<usize, types::Stage>,
    exports: &[(v1::xver_adapt::DecoExport, Option<types::Stage>)],
    step: v1::xver_adapt::UpgradeStep,
) {
    let mut order: Vec<Option<types::Stage>> = Vec::new();
    for (_, st) in exports {
        if !order.contains(st) {
            order.push(*st);
        }
    }
    for st in order {
        let group: Vec<v1::xver_adapt::DecoExport> = exports
            .iter()
            .filter(|(_, s)| *s == st)
            .map(|(e, _)| e.clone())
            .collect();
        splice_staged(
            prelude,
            stages,
            st,
            v1::xver_adapt::deco_upgrade_prelude(&group, step),
        );
    }
}

/// A phase stopwatch that only exists when `$RUSTYFI_TIMING` asked for one.
///
/// The point is the `None` case. `Instant::now()` has no implementation on
/// `wasm32-unknown-unknown` — there is no clock to read, and the call PANICS —
/// so taking one unconditionally meant the browser build could not compile a
/// document at all, however little anyone wanted the timing. An untimed run
/// has no use for the value either way, so not taking it costs nothing.
#[derive(Clone, Copy)]
struct Phase(Option<std::time::Instant>);

impl Phase {
    fn start(timing: bool) -> Self {
        Phase(timing.then(std::time::Instant::now))
    }

    /// Milliseconds since [`Self::start`]. `0.0` when untimed — every caller
    /// is inside an `if timing` that will not print it.
    fn ms(self) -> f64 {
        self.0
            .map(|t| t.elapsed().as_secs_f64() * 1e3)
            .unwrap_or(0.0)
    }
}

/// [`compile_document_cst_with_aux`] told which merged prelude entries came
/// from a file that declared a non-default `@stage:`.
///
/// The loader concatenates every library's prelude into one file, which loses
/// the per-file header; this hands that back, so a `@stage: 0` library is
/// typechecked at stage 0 (where `&e` is legal) while the document around it
/// stays at stage 1 (where it is not).
pub fn compile_document_cst_with_stages(
    file: &rustyfi_syntax::cst::File,
    metrics: &dyn FontMetrics,
    aux: &mut crossref::AuxTable,
    stages: &std::collections::HashMap<usize, types::Stage>,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    let timing = std::env::var_os("RUSTYFI_TIMING").is_some();
    let t = Phase::start(timing);
    let env0 = primitives::base_env();
    // The BRANDED front half lives in its own scope: the `SymbolStore`, the
    // elaborated `Ast<Symbol>` and the typechecker's tables are all dead by
    // the time the fixpoint trials run below.
    //
    // The DE-BRANDED `body` it yields, however, must stay alive until after
    // `eval_document_trials` returns — `Interp::eval_arg` memoizes compiled
    // command arguments by `&Ast` ADDRESS (`eval.rs`'s `arg_cache`), which is
    // sound only while every node it can reach is pinned. Binding it to a
    // local (rather than passing `&debrand(..)` as a temporary) is what pins
    // it.
    let body = {
        let store = symbol::SymbolStore::new();
        let scope = elaborate::Scope::new(&store, env0.names());
        let program = elaborate::elaborate_program_with_stages(file, &scope, stages)?;
        if timing {
            eprintln!("TIMING   elaborate        {:>8.1}ms", t.ms());
        }
        let t = Phase::start(timing);
        typecheck::typecheck(&program)?;
        if timing {
            eprintln!("TIMING   typecheck        {:>8.1}ms", t.ms());
        }
        // The compile membrane: resolve every `Symbol` back to its text, so
        // nothing downstream (the `CompiledExpr`, the per-trial `Env`s,
        // `Value`) carries the store's borrow. See `ast::debrand`.
        ast::debrand(&program.body, &store)
    };
    // Compile the elaborated body into a closure tree ONCE. Each trial below
    // re-runs this same `compiled` against a fresh env + a fresh (except
    // `crossrefs`) `Interp` — safe because `CompiledExpr::run` takes `&self`
    // and re-executes the whole tree from scratch, reproducing upstream's
    // `eval_main i env_freezed ast` per trial (`main.ml:337-397`).
    let t = Phase::start(timing);
    let compiled = compile::compile_program(&body, &env0);
    if timing {
        eprintln!("TIMING   compile-tree     {:>8.1}ms", t.ms());
    }
    eval_document_trials(
        &compiled,
        metrics,
        rustyfi_syntax::RustyfiVersion::V0_0,
        aux,
    )
}

/// The compile-once + fixpoint-trial tail shared by the `V0_0` and `V0_1`
/// entry points (`compile_document_cst_with_trials` above and
/// `compile_document_v1_with_trials` below). The only version-sensitive step
/// is the fresh per-trial env (`primitives::base_env_with_version(version)`);
/// everything else (crossrefs persistence, `fire_hooks`, `DocExtras` attach)
/// is identical regardless of which SATySFi generation produced `compiled`.
fn eval_document_trials(
    compiled: &compile::CompiledExpr,
    metrics: &dyn FontMetrics,
    version: rustyfi_syntax::RustyfiVersion,
    aux: &mut crossref::AuxTable,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    // Seed the fixpoint from the previous run's auxiliary table, if any; if
    // the final trial read a seeded value it never re-derived, redo cold
    // instead (see `CrossRefs::seed_unvalidated`) — this is what keeps a warm
    // build byte-identical to a cold one.
    if !aux.is_empty() {
        let (doc, trials, table, unvalidated) =
            eval_trials_seeded(compiled, metrics, version, aux.clone())?;
        if !unvalidated {
            *aux = table;
            return Ok((doc, trials));
        }
    }
    let (doc, trials, table, _) =
        eval_trials_seeded(compiled, metrics, version, crossref::AuxTable::new())?;
    *aux = table;
    Ok((doc, trials))
}

/// One complete fixpoint run against `seed`. Returns the final cross-reference
/// table alongside the document, plus whether the seed turned out to be
/// load-bearing but unverified ([`CrossRefs::seed_unvalidated`]).
fn eval_trials_seeded(
    compiled: &compile::CompiledExpr,
    metrics: &dyn FontMetrics,
    version: rustyfi_syntax::RustyfiVersion,
    seed: crossref::AuxTable,
) -> Result<(std::rc::Rc<DocumentValue>, u32, crossref::AuxTable, bool), CompileError> {
    let timing = std::env::var_os("RUSTYFI_TIMING").is_some();
    let crossrefs = Rc::new(RefCell::new(CrossRefs::seeded(seed)));
    let mut trials = 0u32;
    loop {
        trials += 1;
        let t_trial = Phase::start(timing);
        // Fresh per trial: `let-mutable` store state resets (== upstream's
        // `env_freezed` re-eval), and a fresh `Interp` resets `hooks`/
        // `images` too — only `crossrefs` is threaded through.
        //
        // The runtime environment is just an empty root frame: the base
        // environment is a COMPILE-time table already folded into the
        // compiled tree, and top-level bindings live in the compiler's slot
        // table, which the spine rewrites as it re-executes each trial.
        // Nothing resolves a name here.
        let env = value::Env::root();
        let mut interp = eval::Interp::new(metrics);
        interp.crossrefs = crossrefs.clone();
        // Threads `version` onto the `Interp` so `read_inline`'s `EmbedMath`
        // fallback arm (no installed math command — unit-test contexts
        // only) can dispatch between `reflect_math_elem`/
        // `reflect_math_elem_v01`.
        interp.version = version;
        let doc = match compiled.run(&env, &mut interp)? {
            Value::Document(doc) => doc,
            other => return Err(CompileError::NotADocument(other.type_name())),
        };
        let t_hooks = Phase::start(timing);
        let run_ms = t_trial.ms();
        // Fire every placed page-break hook now that `break_pages` has given
        // every one of them its final page number/point; hooks mutate
        // `crossrefs` (the only place that seam is legally crossed — see
        // `fire_hooks`'s doc comment).
        fire_hooks(&mut interp, &doc)?;
        if timing {
            eprintln!(
                "TIMING   trial {trials}: run(eval+layout) {:>8.1}ms  fire_hooks {:>6.1}ms",
                run_ms,
                t_hooks.ms()
            );
        }
        let verdict = crossrefs.borrow_mut().verdict();
        match verdict {
            Verdict::NeedsAnotherTrial => continue,
            Verdict::CanTerminate(_) | Verdict::CountMax => {
                // Attach the final trial's accumulated extras. `doc` is
                // usually uniquely held here; if the program's env still
                // holds a clone, fall back to a one-time deep clone.
                let mut final_doc = Rc::try_unwrap(doc).unwrap_or_else(|rc| (*rc).clone());
                final_doc.extras = rustyfi_backend::DocExtras {
                    annotations: std::mem::take(&mut interp.annotations),
                    destinations: std::mem::take(&mut interp.destinations),
                    outline: std::mem::take(&mut interp.outline),
                    page_graphics: std::mem::take(&mut interp.page_graphics),
                    doc_info: interp.doc_info.take(),
                };
                // The DecoId-keyed link/destination side-channel, same
                // timing as `extras` above (only known once `fire_hooks`
                // has run).
                final_doc.reflow_links = std::mem::take(&mut interp.link_decos);
                final_doc.reflow_dests = std::mem::take(&mut interp.dest_decos);
                let refs = crossrefs.borrow();
                return Ok((
                    Rc::new(final_doc),
                    trials,
                    refs.export(),
                    refs.seed_unvalidated(),
                ));
            }
        }
    }
}

/// Compile a loader-resolved SATySFi 0.1 program (`LoadOptions { version:
/// V0_1, .. }`): dependency libraries (`files[..n-1]`, loader
/// dependency-first order) are each lowered to one `TopBinding::Module`
/// (qualified exports — see `v1/lower.rs`'s module doc) via
/// [`v1::lower::lower_file_v1`], the entry (`files[n-1]`, always last —
/// `LoadedProgram::files`'s contract) via [`v1::lower::lower_document_v1`],
/// assembled into ONE synthetic `cst::File` — the same shape the CLI's
/// `merge_program` builds for 0.0.6 — and pushed through the SHARED
/// elaborate -> typecheck(V0_1) -> compile -> fixpoint-eval pipeline.
/// Signature ascriptions (`:>`) are enforced per binding by
/// `v1::module_check::check_program`.
pub fn compile_document_v1(
    files: &[rustyfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    compile_document_v1_with_trials(files, metrics).map(|(doc, _trials)| doc)
}

/// Trial-count-reporting sibling, mirroring
/// `compile_document_cst_with_trials` (same rationale: fixture tests that
/// must see the fixpoint iterate).
pub fn compile_document_v1_with_trials(
    files: &[rustyfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    compile_document_v1_with_aux(files, metrics, &mut crossref::AuxTable::new())
}

/// [`compile_document_v1_with_trials`] threading an AUXILIARY cross-reference
/// table — see [`compile_document_cst_with_aux`]'s doc comment for what
/// seeding `aux` does and why it can't change the output.
pub fn compile_document_v1_with_aux(
    files: &[rustyfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
    aux: &mut crossref::AuxTable,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    use rustyfi_syntax::RustyfiVersion;

    // -- assemble the synthetic cst::File (merge_program's V0_1 analogue) --
    let (entry, deps) = files
        .split_last()
        .expect("loader always yields at least the entry file");
    // Only ever called on the entry: under `compile_document_v1`, the entry
    // is ALWAYS `V0_1` (the loader's own contract — `load_legacy`'s per-file
    // version-detection rule
    // only ever downgrades a DEPENDENCY to `V0_0`, never the entry; see
    // `LoadedFile::version`'s doc comment). A `V0_0` dependency is instead
    // routed through the cross-version splice arm below — it never
    // reaches this helper.
    fn as_v01(f: &rustyfi_loader::LoadedFile) -> &rustyfi_syntax::cst_v1::FileV1 {
        match &f.cst {
            rustyfi_loader::LoadedCst::V0_1(cst) => cst,
            rustyfi_loader::LoadedCst::V0_0(_) => unreachable!(
                "as_v01 called on a V0_0-parsed file — the entry is always \
                 V0_1 under compile_document_v1, and every V0_0 dependency \
                 is routed through the X1 cross-version splice arm instead"
            ),
        }
    }
    // One `SurfaceEnv` threaded across every V0_1 dependency
    // in load order, so a module alias/named-signature reference in a
    // LATER-loaded library can resolve an EARLIER one (`module M =
    // OtherLib`, `:> OtherLib.S`). `build_file_surface` runs (pure `cst_v1`
    // walk, no lowering needed) BEFORE each dep is lowered, so a dep's own
    // internal aliases/named signatures resolve too.
    //
    // `deps` is a MIXED-version list (`LoadedFile::version`). A `V0_1`
    // dep is lowered as usual; a `V0_0` dep contributes its `cst::File.prelude`
    // bindings DIRECTLY (they are already `cst::TopBinding`s — no syntactic
    // bridge needed), positioned dependency-first (loader order).
    // `v006_indices` records which TOP-LEVEL `prelude` slots a V0_0 dep
    // contributed, so `elaborate::elaborate_program_with_versions` (below) can
    // wrap those bindings' RHS in `Ast::VersionScope(V0_0, _)` — the mechanism
    // that makes a version-forked primitive referenced INSIDE such a dependency
    // (`page-break`, `math-*`, …) resolve against `V0_0`'s
    // `PrimDef`/type/runtime-version instead of the merged program's ambient
    // `V0_1`. `dep_csts` collects the V0_1 subset only — `check_program`
    // (below) has no `cst_v1` vocabulary for a `V0_0` file.
    let mut surfaces = v1::surface::SurfaceEnv::default();
    let mut prelude = Vec::new();
    let mut dep_csts: Vec<&rustyfi_syntax::cst_v1::FileV1> = Vec::new();
    let mut v006_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // A spliced 0.0.6 dependency brings its `@stage:` with it, exactly as it
    // would on the 0.0-rooted path -- a `@stage: 0` library must be readable
    // from a 0.1 document too, or the same library compiles from one
    // generation and not the other.
    let mut stages: std::collections::HashMap<usize, types::Stage> =
        std::collections::HashMap::new();
    // Placement state for the forward view-scheduling mechanism: every
    // `deco`/`deco-set`/`paren` export the forward deco/paren wrapper has
    // adapted so far, and whether the 0.0.6-shaped (UNWRAPPED) view of them is
    // the one currently installed at this point in the merged prelude.
    //
    // The forward wrapper installs the 0.1-shaped view by shadowing the
    // export's own name,
    // and a shadow is permanent — the prelude is one flat `Ast::LetIn`
    // chain, and `Ast::VersionScope(V0_0, _)` wraps a binding's RHS, not the
    // continuation after it. So the view is SCHEDULED (as the reverse deco
    // coercion schedules the
    // reverse one): captured once under a private name while the wrapped
    // view is in force, the ORIGINAL restored on entering a 0.0.6-authored
    // block, the wrapped view re-installed on entering a 0.1-authored one
    // (the entry always is, and is always last). Without this, a LATER
    // 0.0.6-authored dependency reads the export at 0.1's shape:
    // `math.satyh`'s `val paren-right : paren` against `latexcmds`'
    // five-argument call, and the `graphics list`/`graphics` mismatch for
    // `deco`.
    //
    // Both transitions are lazy, so a program whose 0.0.6 dependencies
    // never consume each other's crossed exports emits NOTHING extra.
    // `deco_view_captured` counts how many of `deco_exports` already have a
    // private `Capture` of their WRAPPED view in the prelude — an `Install`
    // may only name a view that has been captured.
    //
    // Each export is carried with the DECLARED STAGE of the dependency that
    // exported it, because the glue NAMES that dependency's own binding and
    // `Stage::can_reference` is not symmetric: a `@stage: persistent`
    // dependency may not read a default-stage binding, so a `Restore` of
    // `M.frame` spliced at the default stage would make the very consumer
    // this mechanism exists for fail with a STAGE error instead of a type one.
    let mut deco_exports: Vec<(v1::xver_adapt::DecoExport, Option<types::Stage>)> = Vec::new();
    let mut v006_view_installed = false;
    let mut deco_view_captured: usize = 0;
    for dep in deps {
        match &dep.cst {
            rustyfi_loader::LoadedCst::V0_1(cst) => {
                // Transition back INTO 0.1-authored code: this dependency
                // reads any crossed export at the adapted 0.1 shape, which is
                // what the forward deco/paren wrapper is for.
                if v006_view_installed {
                    splice_upgrade_glue(
                        &mut prelude,
                        &mut stages,
                        &deco_exports[..deco_view_captured],
                        v1::xver_adapt::UpgradeStep::Install,
                    );
                    v006_view_installed = false;
                }
                v1::surface::build_file_surface(cst, &mut surfaces);
                prelude.extend(v1::lower::lower_file_v1_with_surfaces(cst, &surfaces)?);
                dep_csts.push(cst);
            }
            rustyfi_loader::LoadedCst::V0_0(cst) => {
                // Transition INTO 0.0.6-authored code: this
                // dependency means 0.0.6's shape by every name it writes, so
                // any export the forward wrapper has already adapted must
                // read as its
                // UNWRAPPED original here. Deliberately emitted BEFORE `start`
                // is taken below, so this 0.1-authored glue never lands in
                // `v006_indices`/`stages`.
                //
                // The `Capture` rides along with the FIRST such transition
                // rather than being emitted per-dependency: this is the last
                // position at which naming the export's own key still yields
                // the forward wrapper's wrapped view, and emitting it lazily is what keeps a
                // program with no 0.0.6-to-0.0.6 consumption byte-identical to
                // the splice as it stood before this view-scheduling was added.
                if !v006_view_installed || deco_view_captured < deco_exports.len() {
                    if deco_view_captured < deco_exports.len() {
                        splice_upgrade_glue(
                            &mut prelude,
                            &mut stages,
                            &deco_exports[deco_view_captured..],
                            v1::xver_adapt::UpgradeStep::Capture,
                        );
                        deco_view_captured = deco_exports.len();
                    }
                    splice_upgrade_glue(
                        &mut prelude,
                        &mut stages,
                        &deco_exports,
                        v1::xver_adapt::UpgradeStep::Restore,
                    );
                    v006_view_installed = !deco_exports.is_empty();
                }
                // `collect_free_globals` below only checks `free.types`
                // against `forked_type_names` (see the "guard-narrowing"
                // banner comment for which surface sites count and why).
                // Residual gap (no per-name PolyType table): an UNANNOTATED
                // top-level binding whose inferred type carries a forked
                // shape has no syntactic site to catch. Genuine misuse still
                // fails whole-program HM unification at the use site.
                //
                // `math` is representationally IDENTICAL to `V0_1`'s
                // `math-text` (both `Base(MathText)`, `types.rs`; the same
                // shared `Value::MathText`/`Value::Math` runtime rep,
                // value.rs:39-56), so it RELABELS with zero value-level
                // coercion. `reject_type_names()` is `forked_type_names()`
                // PLUS `page`, whose bare name lowers identically under both
                // versions (never appearing in the automatic diff) but whose
                // runtime rep forks (9-ctor ADT vs a tuple), so it is
                // rejected explicitly.
                //
                // `deco`/`deco-set` cross too
                // (`classify_deco_exports`/`deco_coercion_prelude`). Their
                // bare NAME already means the right thing
                // (`typecheck::name_to_mono("deco", V0_1)` is
                // `t_deco(V0_1)` unconditionally), so a textual mention with
                // no attached VALUE (a `type .. = deco` synonym) is already
                // safe. The VALUE needs adapting: a `V0_0` `deco` closure
                // returns `graphics list`
                // (`prim_types::t_graphics_output`/`coerce_graphics_result`)
                // but every `V0_1` call site applying a `deco`
                // (`primitives::apply_deco`) expects a SINGLE `graphics`.
                // For a bare top-level `let-rec name : deco | patbot* = ..`/
                // `: deco-set` export, splice a SECOND, un-scoped binding of
                // the SAME name shadowing the original: it re-applies the
                // still-unshadowed original positionally and unites its
                // `graphics list` into one `graphics` via the real `V0_1`
                // `unite-graphics` (`primitives::prim_unite_graphics`).
                // HM-checked, so a wrapper that doesn't fit fails to
                // typecheck rather than mis-rendering. Every OTHER forked
                // name stays rejected.
                //
                // `reject_type_names_from_v006`, not the shared
                // `reject_type_names`: this dependency's text is
                // 0.0.6-AUTHORED, and `code` forks only in that reading
                // (0.0.6 has no `code` spelling, so `τ code` is an opaque
                // nominal there, while the merged program's hard-coded
                // `V0_1` `Checker` reads the same text as the real staged
                // type). The reverse arm keeps the shared set — a foreign
                // 0.1 dependency's `code` is already in the ambient
                // vocabulary.
                let free = collect_free_globals(&cst.prelude);
                let reject_t = v1::xver_adapt::reject_type_names_from_v006();
                let touched: std::collections::BTreeSet<String> =
                    free.types.intersection(&reject_t).cloned().collect();
                // Anything outside the combined whitelist above (`math`,
                // `deco`, `deco-set`) rejects the WHOLE dependency — no
                // partial acceptance.
                if let Some(name) = touched
                    .iter()
                    .find(|n| !matches!(n.as_str(), "math" | "deco" | "deco-set" | "paren"))
                {
                    return Err(CompileError::CrossVersionUnsupportedName {
                        name: name.clone(),
                        dep: dep.path.display().to_string(),
                        slice: "X3",
                    });
                }
                // A module-scoped deco wrapper lives INSIDE the spliced
                // dependency, hence inside its `VersionScope(V0_0, _)`,
                // where `unite-graphics` does not exist. Bind the V0_1
                // primitive to a plain name FIRST, outside the range
                // `v006_indices` is about to cover, so the scoped wrapper can
                // reach it as an ordinary variable.
                if touched.contains("deco")
                    || touched.contains("deco-set")
                    || touched.contains("paren")
                {
                    let probe = v1::xver_adapt::classify_deco_exports(
                        &cst.prelude,
                        RustyfiVersion::V0_0,
                        RustyfiVersion::V0_1,
                    );
                    if probe
                        .as_ref()
                        .map(|e| v1::xver_adapt::needs_unite_helper(e))
                        == Ok(true)
                    {
                        let helper_start = prelude.len();
                        prelude.extend(v1::xver_adapt::unite_helper_prelude());
                        // Persistent, so the wrapper that calls it can name it
                        // from whatever stage the DEPENDENCY declared: these
                        // helpers are compiler-generated machinery spliced
                        // outside the dependency's own `@stage:` range, and a
                        // `@stage: persistent` dependency may not name a
                        // stage-1 binding (`Stage::can_reference`). Persistent
                        // is the one stage every other stage may reach, which
                        // is exactly the property a generated helper needs.
                        stages.extend(
                            (helper_start..prelude.len())
                                .map(|i| (i, types::Stage::Persistent0)),
                        );
                    }
                }
                let start = prelude.len();
                if touched.is_empty() {
                    // No forked-type-name text anywhere in this dep —
                    // splice verbatim (the GOLDEN/non-regression fast path).
                    prelude.extend(cst.prelude.iter().cloned());
                } else if touched.contains("math") {
                    // Relabel every `math` leaf inside a `type` declaration's
                    // body to `math-text` (the note above) and splice the
                    // adapted prelude. `deco`/`deco-set`, if also touched,
                    // need no textual relabel — so this one call covers the
                    // whole prelude regardless of which combination of the
                    // two is touched.
                    let adapted = v1::xver_adapt::relabel_type_decls(
                        &cst.prelude,
                        RustyfiVersion::V0_0,
                        RustyfiVersion::V0_1,
                    )
                    .map_err(|be| {
                        CompileError::CrossVersionUnsupportedName {
                            name: match &be {
                                v1::xver_adapt::BoundaryError::ForkedTypeExport {
                                    ty_name, ..
                                } => ty_name.clone(),
                            },
                            dep: dep.path.display().to_string(),
                            slice: "X3",
                        }
                    })?;
                    prelude.extend(adapted);
                } else {
                    // Only `deco`/`deco-set` (no `math`) is touched — no
                    // textual relabel needed, splice verbatim (the value-
                    // level coercion, if any, is appended separately below).
                    prelude.extend(cst.prelude.iter().cloned());
                }
                v006_indices.extend(start..prelude.len());
                note_stage(&mut stages, cst, start, prelude.len());

                if touched.contains("deco")
                    || touched.contains("deco-set")
                    || touched.contains("paren")
                {
                    let exports = v1::xver_adapt::classify_deco_exports(
                        &cst.prelude,
                        RustyfiVersion::V0_0,
                        RustyfiVersion::V0_1,
                    )
                    .map_err(|be| {
                        CompileError::CrossVersionUnsupportedName {
                            name: match &be {
                                v1::xver_adapt::BoundaryError::ForkedTypeExport {
                                    ty_name, ..
                                } => ty_name.clone(),
                            },
                            dep: dep.path.display().to_string(),
                            slice: "X3b",
                        }
                    })?;
                    // Deliberately NOT added to `v006_indices` (structural
                    // honesty, not a soundness requirement): this synthetic
                    // code is genuinely `V0_1`-authored (it calls
                    // `unite-graphics`, a `V0_1`-only primitive) — no `V0_0`
                    // `PrimDef` shares this name, so even inside a
                    // `VersionScope(V0_0, _)` the fold cursor would miss and
                    // `compile.rs` would fall back to the eval-time
                    // `env.lookup` against the ambient `V0_1` runtime env.
                    // Two halves: a TOP-LEVEL export is shadowed by a new
                    // top-level binding appended after the dependency
                    // (`deco_coercion_prelude`); a MODULE-scoped one cannot
                    // be (`let Deco.simple-frame` is not syntax), so its
                    // wrapper is appended inside that module's own `decls`
                    // (`inject_module_deco_wrappers`), one scope deeper.
                    v1::xver_adapt::inject_module_deco_wrappers(&mut prelude[start..], &exports);
                    // At the DEPENDENCY's own stage, not the default one: this
                    // top-level wrapper re-applies the dependency's export by
                    // name, and a `@stage: persistent` dependency's binding is
                    // unreadable from a default-stage one (`splice_staged`).
                    // The in-module wrappers above need no such care — they
                    // are spliced INSIDE `prelude[start..]`, already covered by
                    // this dependency's own `note_stage` range.
                    let dep_stage =
                        declared_stage(cst).filter(|s| *s != types::Stage::default());
                    splice_staged(
                        &mut prelude,
                        &mut stages,
                        dep_stage,
                        v1::xver_adapt::deco_coercion_prelude(&exports),
                    );
                    // From here on this export has TWO views in the
                    // program — the wrapper just spliced, and the unwrapped
                    // original the two injectors above kept reachable under
                    // `xver-fwd-orig-`. Record it so the transitions can pick
                    // the right one for whatever block comes next.
                    deco_exports.extend(exports.into_iter().map(|e| (e, dep_stage)));
                }
            }
        }
    }
    // The last (and, for every single-generation dependency set, the ONLY)
    // transition back into 0.1-authored code: the entry itself, which is
    // always `V0_1` here and reads every crossed export at the forward
    // wrapper's adapted
    // shape. Emitted only if some intervening 0.0.6 dependency restored the
    // originals; with no such dependency the whole schedule stays silent.
    if v006_view_installed {
        splice_upgrade_glue(
            &mut prelude,
            &mut stages,
            &deco_exports[..deco_view_captured],
            v1::xver_adapt::UpgradeStep::Install,
        );
    }
    let entry_cst = as_v01(entry);
    let body = v1::lower::lower_document_v1(entry_cst)?;
    let eoi = match entry_cst {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => unreachable!("lower_document_v1 already rejected a Library entry"),
    };
    let file = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(rustyfi_syntax::leaf::KwIn(rustyfi_syntax::Span::default())),
        body: Some(body),
        eoi,
    };

    // -- the shared pipeline, V0_1-tagged (mirrors
    //    compile_document_cst_with_trials line for line) --
    let env0 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    // Branded front half scoped so the store, the `Ast<Symbol>` tree and the
    // module checker's tables are dead before the fixpoint trials run; the
    // de-branded `body` stays pinned for the trials' sake — see
    // `compile_document_cst_with_trials` for both halves of that contract.
    let body = {
        let store = symbol::SymbolStore::new();
        // A spliced `V0_0` dependency may name a `V0_0`-ONLY primitive
        // (`text-in-math`, `get-axis-height`, `math-color`, …). Elaboration
        // resolves names against ONE flat set built from the ambient version,
        // and it runs BEFORE `Ast::VersionScope` can mean anything — the scope
        // wraps an already-elaborated RHS — so such a name was simply
        // "unbound variable" at elaborate time, no matter how correctly the
        // later phases were version-scoped.
        //
        // So when (and ONLY when) a `V0_0` dependency was actually spliced,
        // widen the elaboration name set to the UNION of both versions'
        // primitives. This set answers one question — "is this a known global
        // rather than a free variable?" — and version-correct resolution still
        // happens downstream: `compile.rs`'s fold picks the `V0_0` `PrimDef`
        // inside a `VersionScope(V0_0, _)`, and `typecheck.rs` picks that
        // version's scheme. A pure `V0_1` program takes the other branch and
        // keeps its "unbound variable" diagnostics for `V0_0`-only names.
        let scope_names: Vec<String> = if v006_indices.is_empty() {
            env0.names()
        } else {
            let mut n = env0.names();
            n.extend(primitives::base_env_with_version(RustyfiVersion::V0_0).names());
            n.sort();
            n.dedup();
            n
        };
        let scope = elaborate::Scope::new_with_version(&store, scope_names, RustyfiVersion::V0_1);
        // `v006_indices` is empty whenever no `V0_0` dependency was
        // spliced above, and `elaborate_program_with_versions` then emits no
        // `Ast::VersionScope` node at all — so a `V0_1`-only load's
        // `program`/`compiled` are structurally identical to a plain
        // `elaborate_program`/`compile_program` pair's.
        let program =
            elaborate::elaborate_program_with_versions(
                &file,
                &scope,
                &v006_indices,
                &stages,
                None,
            )?;
        v1::module_check::check_program(&dep_csts, &program)?;
        ast::debrand(&program.body, &store)
    };
    let compiled = if v006_indices.is_empty() {
        compile::compile_program(&body, &env0)
    } else {
        let env0_v006 = primitives::base_env_with_version(RustyfiVersion::V0_0);
        compile::compile_program_xver(&body, &env0, &env0_v006)
    };
    eval_document_trials(&compiled, metrics, RustyfiVersion::V0_1, aux)
}

/// Compile a loader-resolved SATySFi 0.0.6 program (`LoadOptions { version:
/// V0_0, .. }`) whose entry (or one of its native 0.0.6 co-dependencies)
/// `@require:`s at least one **foreign 0.1** package.
///
/// This is the REVERSE of [`compile_document_v1_with_trials`]'s direction, but
/// reuses its exact polarity rather than flipping it: the AMBIENT
/// elaborate/typecheck/compile tag stays `V0_1` (0.1's grammar is a strict
/// syntactic superset of 0.0.6's, so elaborating genuinely 0.0.6-authored code
/// under an ambient `V0_1` scope never rejects it), and it is the
/// 0.0.6-authored code — the ENTRY's own top-level bindings and document tail,
/// plus any native 0.0.6 co-dependency's bindings — that gets wrapped in
/// [`ast::Ast::VersionScope`]`(V0_0, _)`. A foreign 0.1 dependency splices in
/// UNWRAPPED, exactly like a native 0.1 dependency does in
/// `compile_document_v1_with_trials`; its own `:>`-sealed exports are enforced
/// by `v1::module_check::check_program` exactly as for a pure-0.1 consumer.
///
/// A pure-0.0.6 load (no 0.1 dependency) never reaches this function — the
/// CLI/loader only route here once a `V0_0`-rooted load's dependency graph
/// actually contains a `LoadedCst::V0_1` node.
pub fn compile_document_v006_xver(
    files: &[rustyfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    compile_document_v006_xver_with_trials(files, metrics).map(|(doc, _trials)| doc)
}

/// Trial-count-reporting sibling, mirroring `compile_document_v1_with_trials`.
pub fn compile_document_v006_xver_with_trials(
    files: &[rustyfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    compile_document_v006_xver_with_aux(files, metrics, &mut crossref::AuxTable::new())
}

/// [`compile_document_v006_xver_with_trials`] threading an AUXILIARY
/// cross-reference table — see [`compile_document_cst_with_aux`]'s doc
/// comment for what seeding `aux` does and why it can't change the output.
pub fn compile_document_v006_xver_with_aux(
    files: &[rustyfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
    aux: &mut crossref::AuxTable,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    use rustyfi_syntax::RustyfiVersion;

    // The entry is whichever file is a document (`LoadedCst::is_document`) —
    // NOT necessarily `files.last()` (that assumption is specific to
    // `compile_document_v1_with_trials`'s pure-V0_1-entry contract); scan
    // defensively.
    let (entry_idx, entry) = files
        .iter()
        .enumerate()
        .find(|(_, f)| f.cst.is_document())
        .expect("loader validated exactly one document (the entry)");
    let entry_cst = match &entry.cst {
        rustyfi_loader::LoadedCst::V0_0(f) => f,
        rustyfi_loader::LoadedCst::V0_1(_) => unreachable!(
            "compile_document_v006_xver is the V0_0-entry sibling of \
             compile_document_v1 — a V0_1 entry belongs there instead"
        ),
    };

    let mut surfaces = v1::surface::SurfaceEnv::default();
    let mut prelude = Vec::new();
    let mut dep_csts: Vec<&rustyfi_syntax::cst_v1::FileV1> = Vec::new();
    let mut v006_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // A spliced 0.0.6 dependency brings its `@stage:` with it, exactly as it
    // would on the 0.0-rooted path -- a `@stage: 0` library must be readable
    // from a 0.1 document too, or the same library compiles from one
    // generation and not the other.
    let mut stages: std::collections::HashMap<usize, types::Stage> =
        std::collections::HashMap::new();
    // The qualified member keys (`"M.frame"`) this arm rebinds to
    // a version-adapted view, exempted from a SECOND `:>` seal check below.
    let mut xver_shadows: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Placement state for the reverse deco coercion: every `deco`/`deco-set` export crossed so far, and
    // whether the 0.0.6-shaped VIEW of them is the one currently installed at
    // this point in the merged prelude.
    //
    // The prelude is one flat `Ast::LetIn` chain and `Ast::VersionScope(V0_0,
    // _)` wraps a binding's RHS, not the continuation after it, so a
    // rebinding of `M.frame` is visible to EVERYTHING that follows
    // regardless of which generation authored it. A position-indexed view
    // is sufficient because each block spliced below is homogeneous — a
    // `V0_0` dependency's whole `prelude` goes into `v006_indices`, a `V0_1`
    // dependency's whole `lowered` stays out of it, the entry (always
    // 0.0.6-authored) is last — and the loader orders dependencies
    // topologically, so a consumer's block always follows what it
    // `@require:`s. So the coerced view installs lazily on entering a
    // 0.0.6-authored block and is put back on entering a 0.1-authored one.
    //
    // Both transitions are lazy, so the common case (every 0.1 dependency,
    // then the 0.0.6 entry — every bundled package) emits exactly one install
    // and no restore at all.
    let mut deco_exports: Vec<v1::xver_adapt::DecoExport> = Vec::new();
    let mut v006_view_installed = false;

    for (i, dep) in files.iter().enumerate() {
        if i == entry_idx {
            continue;
        }
        match &dep.cst {
            // Native 0.0.6 co-dependency (e.g. the entry ALSO `@require:`s
            // an ordinary 0.0.6 package `list.satyg`-style): splice + wrap.
            // Its VALUE half is unrestricted — every binding here is
            // `Ast::VersionScope(V0_0, _)`-wrapped, so a forked primitive
            // resolves against 0.0.6's own `PrimDef`. Its TYPE-DECLARATION
            // half is NOT unrestricted: the reverse arm's 0.0.6 type-text guard
            // (`guard_v006_type_text`,
            // and the banner above it) refuses/relabels 0.0.6-authored
            // `type` text that a merged program's hard-coded-`V0_1` `Checker`
            // would otherwise re-read with the wrong vocabulary.
            rustyfi_loader::LoadedCst::V0_0(cst) => {
                let adapted = guard_v006_type_text(&cst.prelude, &dep.path)?;
                // Transition INTO 0.0.6-authored code: this dependency reads
                // any crossed `deco` export at 0.0.6's `graphics list` shape.
                // Deliberately BEFORE `start` is taken, so the generated glue
                // (0.1-authored) never lands in `v006_indices`/`stages`.
                if !v006_view_installed && !deco_exports.is_empty() {
                    prelude.extend(v1::xver_adapt::deco_downgrade_prelude(
                        &deco_exports,
                        v1::xver_adapt::DowngradeStep::Install,
                    ));
                    v006_view_installed = true;
                }
                let start = prelude.len();
                prelude.extend(adapted);
                v006_indices.extend(start..prelude.len());
                note_stage(&mut stages, cst, start, prelude.len());
            }
            // Foreign 0.1 dependency: lower (exactly like a native V0_1 dep
            // in `compile_document_v1_with_trials`) and splice UNWRAPPED
            // (ambient V0_1), PLUS the reverse import guard on what it EXPORTS
            // (the forward arm's forked-name guard — narrowed to export
            // position, with the whitelist adaptation — reversed).
            //
            // `Checker.version` for TYPE DECLARATIONS
            // (`v1::module_check::check_program_inner`'s
            // `ck.declare_synonym`/`declare_variant`, and the
            // `base_type_env_with_version` seeding the phase-D spine walk;
            // `module_check.rs:238-239,271`) is HARD-CODED to
            // `RustyfiVersion::V0_1` on BOTH arms — every type declaration in
            // the merged program is read under V0_1 vocabulary
            // unconditionally. That is why the FORWARD arm's
            // `relabel_type_decls(dep.prelude, V0_0, V0_1)` (above) is
            // necessary: a 0.0.6 dependency's own "math" spelling must
            // become "math-text" before it reaches `program.type_decls`, or
            // the V0_1 lookup resolves it to an unrelated unbound nominal.
            //
            // The REVERSE consequence is NOT the naive mirror: a foreign 0.1
            // dependency's own "math-text"/"math-boxes" spelling is ALREADY
            // the ambient vocabulary, so no relabeling is needed or wanted —
            // renaming it to 0.0.6's "math" would corrupt text the
            // hard-coded-V0_1 `Checker` must read natively, turning a
            // working type into an unbound-nominal mismatch. So this arm
            // calls `collect_free_globals` purely as a WHITELIST GUARD: any
            // export-boundary forked type name outside `{"math-text",
            // "math-boxes"}` — a proven-identical-representation set
            // (shared `Value::MathText`/`Value::Math` runtime rep; 0.0.6
            // code has no syntax that could observe the lost distinction) —
            // rejects the WHOLE dependency. False-reject is safe,
            // false-accept is not. `page`/`graphics`/`deco`/`pre-path`/
            // `path`/`image`/`font`/`paren` all still reject in THIS
            // direction too. Past the whitelist, the dependency splices
            // VERBATIM.
            //
            // `deco`/`deco-set` CROSS in this direction too — the
            // reverse mirror of the forward wrapper's `unite-graphics` wrap, coercing the
            // OPPOSITE way. A crossing `V0_1` deco returns a single
            // `graphics`; every `V0_0`-authored consumer call site (and
            // every `V0_0`-scoped `inline-frame-outer`/`inline-frame-
            // breakable` TYPE) expects a `graphics list`, so the wrap is a
            // SINGLETON LIST, `[name p w h d]`. Three steps:
            //
            //   1. `classify_deco_exports_v01_sig` reads the dependency's
            //      PRE-lowering `cst_v1` sig (the ONE textual site 0.1's
            //      grammar can name a `deco` export's type at all — lowering
            //      DROPS `sig_annot` entirely, so this scan must happen here
            //      and not off `lowered`). It descends through nested
            //      `module`/`include` decls and dereferences named signature
            //      references against `surfaces`, which is exactly why
            //      `build_file_surface` above must run FIRST. Anything it
            //      still cannot express — a `paren`, a `deco` buried in a
            //      compound, an OPEN optional row, or a `deco` behind a
            //      functor signature member (whose members have no member
            //      path until some later file APPLIES it) — REJECTS.
            //   2. `deco_downgrade_prelude` generates the coercion glue: a
            //      private `Capture` of the 0.1 original immediately after
            //      the dependency, then an `Install` — a top-level rebinding
            //      of each export's own qualified key (`M.frame`) that
            //      re-applies the captured original positionally and wraps
            //      the result in a singleton list — deferred to the next
            //      0.0.6-authored block, and a `Restore` on the way back
            //      into a 0.1-authored one (see `deco_exports`/
            //      `v006_view_installed` above). None of it is added to
            //      `v006_indices` — this is `V0_1`-authored glue, exactly
            //      like the forward arm's `deco_coercion_prelude`.
            //   3. those qualified keys are collected into `xver_shadows`
            //      and handed to `check_program_with_xver_shadows` below,
            //      which exempts the SECOND `Ast::LetIn` of each from the
            //      `:>` seal re-check (the module's own alias is still
            //      checked; see that function's doc comment for why that
            //      exemption cannot hide a real violation).
            //
            // A BARE `type foo = deco` synonym (no value attached, safe with
            // zero coercion — same reasoning as the forward direction's
            // `type xver-deco-alias = deco`) is UNAFFECTED: it is not a sig
            // `val` item, so this scan never sees it and it splices verbatim.
            rustyfi_loader::LoadedCst::V0_1(cst) => {
                // Transition back INTO 0.1-authored code: this dependency
                // reads any crossed `deco` export at 0.1's own single-
                // `graphics` shape, which is the whole point of the schedule.
                if v006_view_installed {
                    prelude.extend(v1::xver_adapt::deco_downgrade_prelude(
                        &deco_exports,
                        v1::xver_adapt::DowngradeStep::Restore,
                    ));
                    v006_view_installed = false;
                }
                v1::surface::build_file_surface(cst, &mut surfaces);
                let lowered = v1::lower::lower_file_v1_with_surfaces(cst, &surfaces)?;

                let free = collect_free_globals(&lowered);
                let reject_t = v1::xver_adapt::reject_type_names();
                let touched: BTreeSet<String> =
                    free.types.intersection(&reject_t).cloned().collect();
                if let Some(name) = touched.iter().find(|n| {
                    !matches!(n.as_str(), "math-text" | "math-boxes" | "deco" | "deco-set")
                }) {
                    return Err(CompileError::CrossVersionUnsupportedName {
                        name: name.clone(),
                        dep: dep.path.display().to_string(),
                        slice: "X4a",
                    });
                }
                // `touched.contains("deco"/"deco-set")` here can only mean
                // the SAFE, no-coercion-needed case (a bare `type foo =
                // deco` synonym — no value attached; this arm's own doc
                // comment above): a REAL sig-declared VALUE export is
                // invisible to this POST-lowering scan (sig is dropped) and
                // is instead classified by the PRE-lowering scan just below,
                // independently of `touched`.
                let dep_deco_exports = v1::xver_adapt::classify_deco_exports_v01_sig(
                    cst, &surfaces,
                )
                .map_err(|be| CompileError::CrossVersionUnsupportedName {
                    name: match &be {
                        v1::xver_adapt::BoundaryError::ForkedTypeExport { ty_name, .. } => {
                            ty_name.clone()
                        }
                    },
                    dep: dep.path.display().to_string(),
                    slice: "X4b",
                })?;

                prelude.extend(lowered);
                // The private capture goes here and only here: `M.frame` is
                // bound by `lowered` just above, and the 0.1 view is in force
                // at this point (the `Restore` above guarantees it), so this
                // is the one position where naming `M.frame` yields the
                // uncoerced original every later `Install` has to wrap.
                prelude.extend(v1::xver_adapt::deco_downgrade_prelude(
                    &dep_deco_exports,
                    v1::xver_adapt::DowngradeStep::Capture,
                ));
                for exp in &dep_deco_exports {
                    xver_shadows.insert(v1::xver_adapt::deco_export_qualified_name(exp));
                }
                deco_exports.extend(dep_deco_exports);
                dep_csts.push(cst);
            }
        }
    }

    // The last (and, for every bundled package, the ONLY) transition into
    // 0.0.6-authored code: the entry's own prelude AND its document tail are
    // both wrapped in `Ast::VersionScope(V0_0, _)` below, so both read a
    // crossed `deco` export at 0.0.6's `graphics list` shape.
    if !v006_view_installed && !deco_exports.is_empty() {
        prelude.extend(v1::xver_adapt::deco_downgrade_prelude(
            &deco_exports,
            v1::xver_adapt::DowngradeStep::Install,
        ));
    }

    // Entry's OWN top-level lets: splice + wrap, same as a native 0.0.6 dep
    // (a new source of V0_0-tagged items beyond dependency splicing: not just
    // dependencies, but the entry itself) — and through the same
    // 0.0.6-authored type-text guard, for the same reason: the entry's
    // `type` declarations are hoisted into `Program::type_decls` alongside
    // everyone else's and read under the one hard-coded `V0_1` `Checker`,
    // with no `Ast::VersionScope` in reach.
    let entry_adapted = guard_v006_type_text(&entry_cst.prelude, &entry.path)?;
    let entry_start = prelude.len();
    prelude.extend(entry_adapted);
    v006_indices.extend(entry_start..prelude.len());

    let file = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw.clone(),
        body: entry_cst.body.clone(),
        eoi: entry_cst.eoi.clone(),
    };

    // -- the shared pipeline, ambient V0_1-tagged (the elaborate syntax gate
    //    is the one asymmetry — keeping the ambient tag at V0_1 is what lets
    //    genuinely 0.0.6-authored code elaborate unrejected, since 0.1's
    //    grammar is a strict superset) --
    let env0 = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let store = symbol::SymbolStore::new();
    let scope = elaborate::Scope::new_with_version(&store, env0.names(), RustyfiVersion::V0_1);
    // `wrap_body_version = Some(V0_0)`: the ENTRY's own document tail
    // (`file.body`, always 0.0.6-authored here) is wrapped in
    // `Ast::VersionScope(V0_0, _)` too — the one new elaborate.rs
    // capability this reverse direction adds beyond wrapping dependency bindings.
    let program = elaborate::elaborate_program_with_versions(
        &file,
        &scope,
        &v006_indices,
        &stages,
        Some(RustyfiVersion::V0_0),
    )?;
    // `dep_csts` here is the foreign 0.1 dependencies' OWN `cst_v1` trees, so
    // a `:>`-sealed export (e.g. `V01Sealed.t`) is enforced against the WHOLE
    // merged spine exactly as it would be for a pure-0.1 consumer.
    //
    // `xver_shadows` is the ONE thing this arm asks the checker
    // to treat differently, and only for names it has itself just rebound:
    // the exporting module's own alias is still conformance-checked, and
    // only the coercion shadow that FOLLOWS it is exempted from a second
    // check against a signature it deliberately does not match. Empty
    // whenever no 0.1 `deco` export crossed.
    v1::module_check::check_program_with_xver_shadows(&dep_csts, &program, &xver_shadows)?;
    let env0_v006 = primitives::base_env_with_version(RustyfiVersion::V0_0);
    // `v006_indices` is NEVER empty here (the entry's own bindings are
    // always indexed into it above), so this always takes the `_xver` fold
    // path — matching `compile_document_v1_with_trials`'s own `if v006_
    // indices.is_empty() { .. } else { compile_program_xver }` branch,
    // specialized since the `else` arm is the only reachable one.
    // Bound to a local, not passed as a temporary: `Interp::eval_arg`
    // memoizes by `&Ast` address, so the de-branded tree must outlive the
    // trials (see `compile_document_cst_with_trials`).
    let body = ast::debrand(&program.body, &store);
    let compiled = compile::compile_program_xver(&body, &env0, &env0_v006);
    eval_document_trials(&compiled, metrics, RustyfiVersion::V0_0, aux)
}

// ============================================================================
// Forked-name guard: before splicing a V0_0 dependency's `prelude` into a
// V0_1 program (above), walk it for the free (unqualified, unshadowed)
// primitive/type names it references and hard-reject any that is
// version-forked. This is what keeps the splice sound rather than silently wrong —
// see `compile_document_v1_with_trials`'s dep loop for the actual check.
//
// There is no generic CST visitor in this crate (the closest precedent,
// `typecheck.rs`'s `walk_atom`/`walk_expr` quartet, walks only
// `ast::TypeExpr` for tyvars); this is modeled on it but covers the FULL
// `cst::TopBinding`/`ast::Expr`/`ast::Pattern`/`ast::TypeExpr` grammar.
//
// Guard-narrowing: `free.values` is checked against nothing. For
// `free.types` the walk collects EXPORT-POSITION surface sites only — the
// ones a `V0_1` consumer of this dependency can actually observe:
//   - a TOP-LEVEL `TopBinding::LetRec`'s (or `and` sibling's) own `: ty`
//     ascription (`walk_top_binding`'s `LetRec` arm, `boundary = true`);
//   - a `TopBinding::Module`'s `sig` items (`walk_sig_annot` — a `module ..`
//     is only ever a top-level/struct-decl construct, never nested inside an
//     expression, so every site it is walked from is already boundary);
//   - a `TopBinding::Type` declaration's body (`walk_type_decl`, kept
//     UNCONDITIONALLY boundary: a `type` declaration's ctor payload/synonym
//     body is registered ONCE under the merged program's single ambient
//     `V0_1` `Checker`, never inside an `Ast::VersionScope`, and a flat
//     splice makes the declared name visible to the consumer — so "unused
//     within this dependency" is not provable-safe here).
// An INTERNAL `Expr::LetRecIn` ascription is SKIPPED (`boundary = false`,
// `walk_expr`'s `LetRecIn` arm); it is the one place a forked type name can
// appear buried in an expression body in this port's 0.0.6 grammar, which has
// no local-lambda-parameter or local-`type` ascription syntax at all. See
// `walk_rec_binding_body`'s doc comment for the mechanism and the residual
// risk.
// ============================================================================

/// The free, unqualified global names a spliced V0_0 dependency's
/// `prelude` references, split by namespace (values/commands vs. types)
/// because they are checked against DIFFERENT forked-name sets. See
/// `collect_free_globals`'s doc comment for the walk itself.
#[derive(Default, Debug)]
struct FreeGlobals {
    /// Value-position occurrences that could resolve to `base_env`:
    /// `Atomic::Var`/`Ctor`/`OpRef`/`Command`, the `Plain` arm of an
    /// `AnyHorz`/`Vert`/`MathCmdTok` reference, and an unqualified
    /// `…Elem::Embed`/`MathBot::Embed`. (No longer checked against
    /// anything — collected for completeness/tests only, see this module's
    /// banner comment above `walk_top_binding`.)
    values: BTreeSet<String>,
    /// EXPORT-BOUNDARY type-position occurrences only (see this
    /// module's "Guard-narrowing" banner comment above): `TypeAtom::Name`
    /// and the `ctor` of `TypeApp::Applied`, collected ONLY from a top-level
    /// binding's own ascription, a module's `sig`, or a `type` declaration's
    /// body — never from a purely-internal/local ascription.
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

/// Enumerate the *free, unqualified* global names a spliced V0_0
/// dependency references — `TopBinding`/`ast::Expr`/`ast::Pattern`/
/// `ast::TypeExpr`, each threading a binder scope stack so a locally-bound
/// name shadows a primitive of the same name (per `XverScope`'s doc
/// comment). A module-qualified reference (`Atomic::VarWithMod`,
/// `\Mod.cmd`/`+Mod.cmd`/`#Mod.var`) is deliberately SKIPPED: a primitive or
/// builtin type is only ever reachable by a BARE name, so a qualified
/// reference resolves inside a module and can never collide with a forked
/// primitive (0.0.6 has no qualified *type*-name form at all, so every type
/// reference is in scope for this check).
fn collect_free_globals(prelude: &[rustyfi_syntax::cst::TopBinding]) -> FreeGlobals {
    let mut out = FreeGlobals::default();
    let mut scope = XverScope::default();
    for tb in prelude {
        walk_top_binding(tb, &mut scope, &mut out);
    }
    out
}

fn walk_top_binding(
    tb: &rustyfi_syntax::cst::TopBinding,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::TopBinding;
    match tb {
        // Recursive: every clause's own name is bound BEFORE any clause body
        // is walked (and stays bound for every sibling `and` clause too).
        TopBinding::LetRec { first, ands, .. } => {
            scope.push_value(&first.name.name);
            for and in ands {
                scope.push_value(&and.binding.name.name);
            }
            // TOP-LEVEL — a consumer-observable export; `boundary = true`
            // (this binding's own `: ty` ascription IS export-position
            // text).
            walk_rec_binding_body(first, true, scope, out);
            for and in ands {
                walk_rec_binding_body(&and.binding, true, scope, out);
            }
        }
        TopBinding::Let(tl) => {
            // TOP-LEVEL, so this binding's own `: ty` ascription is
            // export-position text, exactly as `LetRec`'s is. Skipping it
            // would let `let x : page = ...` cross silently while `type
            // alias = page` was rejected — the same forked name, caught or
            // not depending on which way the package spelled it.
            if let Some(asc) = &tl.ascription {
                walk_type_expr(&asc.ty, scope, out);
            }
            let mark = scope.mark();
            for p in &tl.params {
                walk_param_binder(p, scope, out);
            }
            walk_expr(&tl.value, scope, out);
            scope.truncate_to(mark);
            scope.push_value(&tl.name.name);
        }
        TopBinding::LetPattern { value, .. } => {
            // Destructuring `let pat = value`: only the scrutinee references
            // free globals. The pattern-bound names become new bindings; not
            // pushing them here is sound (this walk over-approximates the free
            // set — see the module banner).
            walk_expr(value, scope, out);
        }
        TopBinding::LetInline {
            ctx,
            cmd,
            params,
            value,
            ..
        } => {
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
        TopBinding::LetBlock {
            ctx,
            cmd,
            params,
            value,
            ..
        } => {
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
        TopBinding::LetMath {
            cmd, params, value, ..
        } => {
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
///
/// `boundary`: whether THIS `RecBinding` is a TOP-LEVEL,
/// consumer-observable export (`TopBinding::LetRec`/its `and` siblings —
/// `true`) or a purely LOCAL binding nested inside another binding's
/// expression body (`Expr::LetRecIn` — `false`). Only when `boundary` is
/// true does the binding's OWN `: ty` ascription get walked into
/// `out.types` — see the "Guard-narrowing" banner above
/// `collect_free_globals`.
fn walk_rec_binding_body(
    rb: &rustyfi_syntax::cst::ast::RecBinding,
    boundary: bool,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    if boundary {
        if let Some(asc) = &rb.ascription {
            walk_type_expr(&asc.ty, scope, out);
        }
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

fn walk_param_binder(
    p: &rustyfi_syntax::cst::ast::Param,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::Param;
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
fn walk_pattern_binder(
    pat: &rustyfi_syntax::cst::ast::Pattern,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_patcons_binder(&pat.head, scope, out);
    if let Some(ac) = &pat.as_clause {
        scope.push_value(&ac.name.name);
    }
}

fn walk_patcons_binder(
    pc: &rustyfi_syntax::cst::ast::PatCons,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_patbot_binder(&pc.head, scope, out);
    for seg in &pc.tail {
        walk_patbot_binder(&seg.tail, scope, out);
    }
}

fn walk_patbot_binder(
    pb: &rustyfi_syntax::cst::ast::PatBot,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::PatBot;
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

// ============================================================================
// The REVERSE arm's guard on **0.0.6-authored** type text
// (`compile_document_v006_xver_with_aux`'s `LoadedCst::V0_0` branch and the
// entry's own prelude).
//
// **The misreading**, reached from the other side of the forward arm's: a
// merged cross-version program has exactly one `Checker`, hard-coded to
// `V0_1` (`v1::module_check::check_program_inner`'s `ck.set_version`) on
// BOTH arms, because `elaborate` hoists every `type` declaration out of the
// `Ast` spine into `Program::type_decls`/`synonym_decls` — never inside an
// `Ast::VersionScope`. Forward, the 0.0.6 text re-read under 0.1's
// vocabulary is a spliced dependency's; reverse, it is the ENTRY's own
// prelude plus every native 0.0.6 co-dependency — potentially the whole
// 0.0.6 corpus.
//
// So `math` takes the **same** relabel here as forward, `math` ->
// `math-text` (`xver_adapt::relabel_type_decls(_, V0_0, V0_1)`), NOT the
// mirror `math-text` -> `math`: the target vocabulary is `V0_1` either way.
// (`relabel_or_reject_name`'s mirror arm is deliberately not wired to the
// reverse arm's `LoadedCst::V0_1` branch — a foreign 0.1 dependency's text
// is already in the ambient vocabulary.)
//
// **Why this scan is narrower than `collect_free_globals`.** The forward
// arm over-approximates on purpose, also collecting from a `let-rec`'s
// `: ty` ascription and a `module .. : sig .. end`'s `val` items — both
// parsed and then ignored by `elaborate.rs`, so over-rejecting on them only
// costs a 0.1 document a 0.0.6 package it could have had. Reversed, the
// same over-approximation would be WRONG, not conservative: the bundled
// 0.0.6 corpus writes forked names in exactly those decorative positions
// all the time (`vdecoset.satyh`'s `val paper : deco-set`, `math.satyh`'s
// `direct \frac : [math; math] math-cmd`), so rejecting on them would
// refuse ordinary 0.0.6 documents for text no phase reads. This walk
// instead collects from `TopBinding::Type` bodies alone (recursing through
// `TopBinding::Module`'s nested `decls`) — exactly the site set
// `xver_adapt::relabel_type_decls` rewrites, and the text that reaches
// `declare_variant`/`declare_synonym`.
//
// **What is refused.** `reject_type_names_from_v006()` — the same
// producer-keyed set the forward arm's `V0_0` branch uses, so `code`
// refuses here too (a foreign 0.1 dependency's `code`, the reverse arm's
// OTHER branch, keeps the shared `reject_type_names()` and does not). The
// whitelist is `{"math"}` alone: this branch has no
// `classify_deco_exports`/`deco_coercion_prelude` pairing to make a
// `deco`/`deco-set`/`paren` mention safe, and the 0.1 reading of those names
// is wrong for a 0.0.6-authored consumer anyway (0.0.6's `deco` returns
// `graphics list`; `name_to_mono("deco", V0_1)` types it as a single
// `graphics`). `page` is the sharp one: its bare name lowers to the
// same nominal `Variant("page",[])` under both versions, so a mismatch is
// not a type error at all — a 9-ctor `Value::Ctor` meeting a `length *
// length` `Value::Product`.
// ============================================================================

/// The free type names a 0.0.6-authored `prelude`'s `type` DECLARATIONS
/// mention — the whole of that prelude's text a merged cross-version
/// program's single hard-coded-`V0_1` `Checker` actually reads (see this
/// module's banner above for why the decorative
/// ascription/`sig` sites are deliberately NOT collected here, though
/// `collect_free_globals` does collect them for the forward arm).
fn collect_type_decl_globals(
    prelude: &[rustyfi_syntax::cst::TopBinding],
) -> std::collections::BTreeSet<String> {
    let mut out = FreeGlobals::default();
    let mut scope = XverScope::default();
    for tb in prelude {
        walk_type_decls_only(tb, &mut scope, &mut out);
    }
    out.types
}

fn walk_type_decls_only(
    tb: &rustyfi_syntax::cst::TopBinding,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::TopBinding;
    match tb {
        TopBinding::Type(td) => {
            walk_type_decl(td, scope, out);
            scope.push_type(&td.name.name);
        }
        // A nested `type` declaration is hoisted into the SAME
        // `Program::type_decls` as a top-level one (`elaborate::
        // walk_bindings` threads one `type_decls` sink through every level),
        // so it is read under the same hard-coded `V0_1` `Checker` and must
        // be scanned too. Its locally-declared names stay local, matching
        // `walk_top_binding`'s own `Module` arm.
        TopBinding::Module { decls, .. } => {
            let mark = scope.mark();
            for d in decls {
                walk_type_decls_only(&d.0, scope, out);
            }
            scope.truncate_to(mark);
        }
        _ => {}
    }
}

/// Check one 0.0.6-authored `prelude` on the REVERSE arm and
/// return the bindings to splice — relabeled (`math` -> `math-text`) when
/// that is all it touches, cloned verbatim when it touches nothing, and a
/// `CompileError::CrossVersionUnsupportedName` naming the offending type
/// otherwise. `path` is the file the text was authored in (the 0.0.6 entry,
/// or a native 0.0.6 co-dependency); the resulting error records which
/// DIRECTION refused, since the forward arm's guard checks the same
/// producer-keyed set under its own tag.
fn guard_v006_type_text(
    prelude: &[rustyfi_syntax::cst::TopBinding],
    path: &std::path::Path,
) -> Result<Vec<rustyfi_syntax::cst::TopBinding>, CompileError> {
    use rustyfi_syntax::RustyfiVersion;
    let reject_t = v1::xver_adapt::reject_type_names_from_v006();
    let touched: BTreeSet<String> = collect_type_decl_globals(prelude)
        .intersection(&reject_t)
        .cloned()
        .collect();
    // `math` is the whole whitelist here — see the banner above.
    if let Some(name) = touched.iter().find(|n| n.as_str() != "math") {
        return Err(CompileError::CrossVersionUnsupportedName {
            name: name.clone(),
            dep: path.display().to_string(),
            slice: "X4c",
        });
    }
    if touched.is_empty() {
        // Byte-identical to the `prelude.extend(cst.prelude.iter()
        // .cloned())` fast path every non-`math` 0.0.6 file takes.
        return Ok(prelude.to_vec());
    }
    v1::xver_adapt::relabel_type_decls(prelude, RustyfiVersion::V0_0, RustyfiVersion::V0_1).map_err(
        |be| CompileError::CrossVersionUnsupportedName {
            name: match &be {
                v1::xver_adapt::BoundaryError::ForkedTypeExport { ty_name, .. } => ty_name.clone(),
            },
            dep: path.display().to_string(),
            slice: "X4c",
        },
    )
}

fn walk_type_decl(
    td: &rustyfi_syntax::cst::TypeDecl,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_type_decl_body(&td.body, scope, out);
    for a in &td.ands {
        walk_type_decl_body(&a.body, scope, out);
    }
}

fn walk_type_decl_body(
    body: &rustyfi_syntax::cst::TypeDeclBody,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::TypeDeclBody;
    match body {
        TypeDeclBody::Variant { first, rest, .. } => {
            walk_variant_def(first, scope, out);
            for bv in rest {
                walk_variant_def(&bv.def, scope, out);
            }
        }
        TypeDeclBody::Synonym(ty) => walk_type_expr(ty, scope, out),
    }
}

fn walk_variant_def(
    vd: &rustyfi_syntax::cst::VariantDef,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    // `vd.ctor` DECLARES a new constructor — not a reference, nothing to
    // emit for it.
    if let Some(of_ty) = &vd.of_ty {
        walk_type_expr(&of_ty.ty, scope, out);
    }
}

fn walk_sig_annot(
    sig: &rustyfi_syntax::cst::SigAnnot,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::SigItem;
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

fn walk_expr(e: &rustyfi_syntax::cst::ast::Expr, scope: &mut XverScope, out: &mut FreeGlobals) {
    use rustyfi_syntax::cst::ast::Expr;
    match e {
        Expr::LetRecIn {
            first, ands, body, ..
        } => {
            let mark = scope.mark();
            scope.push_value(&first.name.name);
            for and in ands {
                scope.push_value(&and.binding.name.name);
            }
            // INTERNAL — a local binding nested inside some enclosing
            // binding's own body; `boundary = false` (this `let rec`'s
            // OWN `: ty` ascription is not, by itself, any export's
            // observable signature text — see `walk_rec_binding_body`'s doc
            // comment).
            walk_rec_binding_body(first, false, scope, out);
            for and in ands {
                walk_rec_binding_body(&and.binding, false, scope, out);
            }
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::LetIn {
            name,
            params,
            value,
            body,
            ..
        } => {
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
        Expr::LetPatternIn {
            pat, value, body, ..
        } => {
            walk_expr(value, scope, out);
            let mark = scope.mark();
            walk_pattern_binder(&pat.0, scope, out);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
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
        Expr::FunRows {
            opts, param, body, ..
        } => {
            let mark = scope.mark();
            for e in &opts.entries {
                scope.push_value(&e.var.name);
            }
            walk_patbot_binder(param, scope, out);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::Match {
            scrutinee,
            first,
            rest,
            ..
        } => {
            walk_expr(scrutinee, scope, out);
            walk_match_arm(first, scope, out);
            for ba in rest {
                walk_match_arm(&ba.arm, scope, out);
            }
        }
        Expr::LetMutableIn {
            name, init, body, ..
        } => {
            walk_expr(init, scope, out);
            let mark = scope.mark();
            scope.push_value(&name.name);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::LetMathIn {
            cmd,
            params,
            value,
            body,
            ..
        } => {
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

fn walk_match_arm(
    arm: &rustyfi_syntax::cst::ast::MatchArm,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    let mark = scope.mark();
    walk_pattern_binder(&arm.pat.0, scope, out);
    if let Some(g) = &arm.guard {
        walk_expr(&g.cond.0, scope, out);
    }
    walk_expr(&arm.body.0, scope, out);
    scope.truncate_to(mark);
}

fn walk_opchain(
    oc: &rustyfi_syntax::cst::ast::OpChain,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_appexpr(&oc.head, scope, out);
    for r in &oc.tail {
        walk_appexpr(&r.rhs, scope, out);
    }
    if let Some(bt) = &oc.before {
        walk_expr(&bt.body.0, scope, out);
    }
}

fn walk_appexpr(
    ae: &rustyfi_syntax::cst::ast::AppExpr,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_atomic(&ae.head, scope, out);
    // `head_accesses`: `#label` record-field accesses — field labels, not
    // globals, skip.
    for arg in &ae.args {
        walk_apparg(arg, scope, out);
    }
}

fn walk_apparg(a: &rustyfi_syntax::cst::ast::AppArg, scope: &mut XverScope, out: &mut FreeGlobals) {
    use rustyfi_syntax::cst::ast::AppArg;
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

fn walk_atomic(a: &rustyfi_syntax::cst::ast::Atomic, scope: &mut XverScope, out: &mut FreeGlobals) {
    use rustyfi_syntax::cst::ast::Atomic;
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

fn walk_any_horz_cmd_ref(
    n: &rustyfi_syntax::leaf::AnyHorzCmdTok,
    scope: &XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::leaf::AnyHorzCmdTok;
    match n {
        AnyHorzCmdTok::Plain(t) => emit_value(scope, out, &t.name),
        AnyHorzCmdTok::Mod(_) => {} // qualified — skip
    }
}

fn walk_any_vert_cmd_ref(
    n: &rustyfi_syntax::leaf::AnyVertCmdTok,
    scope: &XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::leaf::AnyVertCmdTok;
    match n {
        AnyVertCmdTok::Plain(t) => emit_value(scope, out, &t.name),
        AnyVertCmdTok::Mod(_) => {} // qualified — skip
    }
}

fn walk_any_math_cmd_ref(
    n: &rustyfi_syntax::leaf::AnyMathCmdTok,
    scope: &XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::leaf::AnyMathCmdTok;
    match n {
        AnyMathCmdTok::Plain(t) => emit_value(scope, out, &t.name),
        AnyMathCmdTok::Mod(_) => {} // qualified — skip
    }
}

fn walk_paren_body(
    pb: &rustyfi_syntax::cst::ast::ParenBody,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_expr(&pb.first.0, scope, out);
    for ce in &pb.rest {
        walk_expr(&ce.value.0, scope, out);
    }
}

fn walk_record_body(
    rb: &rustyfi_syntax::cst::ast::RecordBody,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::RecordBody;
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

fn walk_inline_elem(
    el: &rustyfi_syntax::cst::ast::InlineElem,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::InlineElem;
    match el {
        InlineElem::Char(_)
        | InlineElem::CodeText(_)
        | InlineElem::Space(_)
        | InlineElem::Break(_) => {}
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

fn walk_block_elem(
    el: &rustyfi_syntax::cst::ast::BlockElem,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::BlockElem;
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

fn walk_cmd_tail(
    t: &rustyfi_syntax::cst::ast::CmdTail,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::CmdTail;
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

fn walk_math_elem(
    m: &rustyfi_syntax::cst::ast::MathElemCst,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_math_bot(&m.base, scope, out);
    for s in &m.scripts {
        walk_math_script(s, scope, out);
    }
}

fn walk_math_script(
    s: &rustyfi_syntax::cst::ast::MathScript,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::MathScript;
    match s {
        MathScript::Super { group, .. } | MathScript::Sub { group, .. } => {
            walk_math_group_arg(group, scope, out)
        }
        MathScript::Primes(_) => {}
    }
}

fn walk_math_group_arg(
    g: &rustyfi_syntax::cst::ast::MathGroupArg,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::MathGroupArg;
    match g {
        MathGroupArg::Group { elems, .. } => {
            for m in elems {
                walk_math_elem(&m.0, scope, out);
            }
        }
        MathGroupArg::Bot(b) => walk_math_bot(b, scope, out),
    }
}

fn walk_math_bot(
    b: &rustyfi_syntax::cst::ast::MathBot,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::MathBot;
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

fn walk_math_arg(
    a: &rustyfi_syntax::cst::ast::MathArg,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::MathArg;
    match a {
        MathArg::Optional { body, .. } => walk_math_arg_body(body, scope, out),
        MathArg::Omission(_) => {}
        MathArg::Plain(body) => walk_math_arg_body(body, scope, out),
    }
}

fn walk_math_arg_body(
    b: &rustyfi_syntax::cst::ast::MathArgBody,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::MathArgBody;
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

fn walk_type_expr(
    te: &rustyfi_syntax::cst::ast::TypeExpr,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::TypeExpr;
    match te {
        TypeExpr::Fun { opts, dom, cod, .. } => {
            for o in opts {
                walk_type_prod(&o.ty, scope, out);
            }
            walk_type_prod(dom, scope, out);
            walk_type_expr(cod, scope, out);
        }
        TypeExpr::Atom(prod) => walk_type_prod(prod, scope, out),
        TypeExpr::OptRowFun {
            opt_dom, dom, cod, ..
        } => {
            for e in &opt_dom.entries {
                walk_type_expr(&e.ty.0, scope, out);
            }
            walk_type_prod(dom, scope, out);
            walk_type_expr(cod, scope, out);
        }
    }
}

fn walk_type_prod(
    tp: &rustyfi_syntax::cst::ast::TypeProd,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    walk_type_app(&tp.first, scope, out);
    for st in &tp.rest {
        walk_type_app(&st.ty, scope, out);
    }
}

fn walk_type_app(
    ta: &rustyfi_syntax::cst::ast::TypeApp,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    // Every atom of a postfix application `arg1 … ctor` — including the final
    // constructor — is a `TypeAtom`, and `walk_type_atom` already emits a bare
    // `Name` (and skips a module-qualified `NameMod`) as an
    // export-boundary type reference, so walking the whole run reproduces the
    // old per-arg-then-ctor behavior exactly.
    walk_type_atom(&ta.head, scope, out);
    for a in &ta.rest {
        walk_type_atom(a, scope, out);
    }
}

fn walk_type_atom(
    atom: &rustyfi_syntax::cst::ast::TypeAtom,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use rustyfi_syntax::cst::ast::TypeAtom;
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
        // `Mod.t` — already qualified, not a free unqualified global.
        TypeAtom::NameMod(_) => {}
        TypeAtom::RecordOpen { inner, .. } => {
            for f in &inner.fields {
                walk_type_expr(&f.ty.0, scope, out);
            }
        }
    }
}

/// One `block-frame-breakable` frame currently between its `FrameStart`/
/// `FrameEnd` markers on the page being walked.
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
    /// `true` once this frame has already emitted a head (`decoH`) or middle
    /// (`decoM`) fragment on an EARLIER page — i.e. its `FrameStart` landed on
    /// a previous page and it is still open. Drives the S/H/M/T choice: a
    /// non-carried frame closing on its start page fires `decoS`; a carried one
    /// fires `decoT`. At each page boundary a still-open frame fires `decoH`
    /// (first spanned page) or `decoM` (subsequent) and its per-page extent is
    /// reset. `false` for the common single-page frame (unchanged behaviour).
    carried: bool,
}

/// The inline twin of [`OpenFrame`]: one `inline-frame-breakable` that is open
/// at some point during the placed-line walk. Where a block frame's fragments
/// are delimited by PAGE boundaries, an inline frame's are delimited by LINE
/// boundaries (upstream `append_framed_lines`, `lineBreak.ml:695`), so one of
/// these can open and close within a single line — the common case, and the
/// one that fires `decoS`.
#[derive(Clone)]
struct OpenInlineFrame {
    id: DecoId,
    /// Left edge of the fragment currently being accumulated, in absolute page
    /// coordinates: the start marker's own x on the line that opened the
    /// frame, and the line's own left edge on every continuation line.
    x: Length,
    /// Baseline of the line the current fragment sits on.
    baseline_y: Length,
    /// The frame's padded vertical extent, carried on its markers — see
    /// `PureHorzBox::InlineFrameMarker` for why it is the whole frame's rather
    /// than this fragment's.
    height: Length,
    depth: Length,
    /// `true` once an earlier fragment of this frame has already fired, i.e.
    /// the frame really did split — same S/H/M/T choice as `OpenFrame`'s
    /// `carried` field above.
    carried: bool,
}

/// The absolute x just past a placed line's last box — where a fragment that
/// runs off the end of this line has to stop.
///
/// Uses each box's NATURAL width rather than its justified advance: the only
/// boxes whose two differ are glue, `line_content` trims a line's trailing
/// glue away, and an `inline-fil` that survives (the `… ++ inline-fil`
/// flush-left idiom) is exactly the case where the fragment should stop at the
/// ink rather than at the stretched fil.
fn placed_line_right_edge(line: &rustyfi_backend::PlacedLine) -> Length {
    let mut edge = Length::ZERO;
    for (dx, bx) in &line.contents {
        let right = *dx + bx.natural_width();
        if right > edge {
            edge = right;
        }
    }
    line.x + edge
}

/// Fire one placed `inline-frame-breakable` fragment's decoration
/// (`decoS`/`decoH`/`decoM`/`decoT` picked by `deco_idx` — 0/1/2/3,
/// evalUtil.ml:169 `get_decoset` order), spanning `frame.x` to `right`.
///
/// The vertical padding is already folded into `frame.height`/`frame.depth`
/// (the markers carry the padded extent), so unlike the block twin this takes
/// no per-fragment pad selection: upstream's `append_vert_padding`
/// (`lineBreak.ml:74`) applies `paddingT`/`paddingB` to EVERY fragment of an
/// inline frame, not just the first and last — the split is horizontal, so
/// each fragment has its own full-height top and bottom edge.
fn fire_inline_frame_fragment(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    page: usize,
    frame: &OpenInlineFrame,
    right: Length,
    deco_idx: usize,
) -> Result<(), eval::EvalError> {
    let (deco, deco_version) = match &interp.decos[frame.id.0] {
        eval::DecoEntry::InlineBreakable {
            decoset, version, ..
        } => (decoset[deco_idx].clone(), *version),
        _ => {
            return eval::eval_error("BUG: non-breakable deco behind an inline frame marker");
        }
    };
    let width = right - frame.x;
    let pt = (frame.x, doc.geometry.paper_height - frame.baseline_y);
    // See the block-frame call site's identical comment — `annot.satyh`'s
    // `\href` fires `register-link-to-uri` from exactly this closure.
    interp.current_deco_id = Some(frame.id);
    let gr = primitives::apply_deco(
        interp,
        deco_version,
        deco,
        pt,
        width,
        frame.height,
        frame.depth,
    )?;
    interp.current_deco_id = None;
    interp.page_graphics[page].extend(gr);
    Ok(())
}

/// Fire every placed page-break hook and decoration, in document order, now
/// that final page numbers and points are known. This is the port's
/// callback architecture: `make_hook` + `handlePdf.ml:234/337`'s invocation
/// (hooks) and `EvHorzFrame`/`EvVertFrame` (decos), relocated to the one
/// place that legally holds `&mut Interp` — the backend produced the
/// geometry (POD `HookId`/`DecoId` tokens riding inside placed boxes, per
/// `hbox.rs`); this reads them back and re-enters the evaluator.
///
/// Sets `interp.current_page` to `Some(i)` for the duration of page `i`'s
/// walk (the "during page break" window: `register-destination`/
/// `register-link-to-*` — called directly by a hook or, more commonly,
/// transitively by a fired deco closure, e.g. `annot.satyh`'s `\href` —
/// only succeed inside this window) and back to `None` once every page is
/// done.
///
/// **Known scope cuts** (documented deviations):
/// - Frames nested inside a `Tabular` cell or an `EmbeddedBlock`'s stacked
///   lines are NOT discovered by this walk — their placed positions would
///   need the writers' cell/stack arithmetic replicated lang-side. No
///   bundled package puts an `\href`/frame inside one today.
/// - A `block-frame-breakable` frame whose `FrameStart` and `FrameEnd` land
///   on DIFFERENT pages now fires per-page fragments: `decoS` if it opens and
///   closes on one page, else `decoH` on its first (opening) page, `decoM` on
///   each fully-contained middle page, and `decoT` on its closing page — the
///   `pageBreak.ml` fragment split. Each fragment's rect spans only that
///   page's content extent, plus the frame's TOP pad (which `chop_page`
///   re-applies on every continuation page, `pageBreak.ml:322`) and, on the
///   tail/single fragment, the bottom pad. This is
///   what lets `figbox`'s `+fig-on-right`/`+fig-on-left` (which draw their
///   image in `decoH`) render on figures whose surrounding text wraps across a
///   page break.
///
/// `pub` (rather than crate-private) so unit tests can drive it directly
/// against a hand-built `DocumentValue`, without going through a full
/// `compile_document_cst` fixpoint.
pub fn fire_hooks(interp: &mut eval::Interp, doc: &DocumentValue) -> Result<(), eval::EvalError> {
    interp.page_graphics = doc.pages.iter().map(|_| Vec::new()).collect();
    let mut next_open_seq: usize = 0;
    // Frames persist ACROSS pages: a `block-frame-breakable` whose `FrameStart`
    // and `FrameEnd` straddle a page break stays in `open` between pages so its
    // head/middle fragments fire at each boundary and its tail fires when the
    // `FrameEnd` finally arrives. Single-page frames are pushed and removed
    // within one page's walk.
    let mut open: Vec<OpenFrame> = Vec::new();
    // Inline frames persist across LINES the same way, and across pages too
    // (the line a frame continues onto can be the first line of the next
    // page), so this lives outside the page loop as well.
    let mut open_inline: Vec<OpenInlineFrame> = Vec::new();
    for (i, page) in doc.pages.iter().enumerate() {
        interp.current_page = Some(i);
        let page_number = (i + 1) as i64; // 1-based, = pbinfo#page-number
                                          // Frames carried over from a previous page start a fresh per-page
                                          // extent: their fragment on THIS page spans only this page's lines.
        for f in &mut open {
            f.top = None;
            f.bottom = None;
        }
        // (open_seq, graphics) per block-frame fragment fired on this page,
        // sorted by open order before being appended to the page's underlay
        // — see the doc comment on the ordering this preserves.
        let mut closings: Vec<(usize, Vec<GraphicsElem>)> = Vec::new();

        for (line_idx, line) in page.lines.iter().enumerate() {
            // An inline frame that was still open when the previous line ended
            // continues here: re-anchor it to THIS line's left edge and
            // baseline, so its next fragment measures from where it resumes.
            // Body lines only — the header and footer are appended after the
            // columns and belong to their own line-break runs, so a frame
            // straddling the body/header boundary must not paint across them
            // (the same reason `Page::body_lines` exists for block frames).
            let is_body = line_idx < page.body_lines;
            if is_body {
                for f in &mut open_inline {
                    f.x = line.x;
                    f.baseline_y = line.baseline_y;
                }
            }
            for (dx, bx) in &line.contents {
                match bx {
                    PureHorzBox::HookPageBreak { id } => {
                        fire_page_break_hook(
                            interp,
                            doc,
                            page_number,
                            line.x + *dx,
                            line.baseline_y,
                            *id,
                        )?;
                    }
                    // `Tabular`/`Graphics` for the same reason `Frame` is here:
                    // a cell's boxes, and the inline run a `draw-text` carries,
                    // never appear in the page flow, so a `\href` or a
                    // `hook-page-break` inside one is only reachable through
                    // this recursion.
                    PureHorzBox::Frame { .. }
                    | PureHorzBox::Tabular(_)
                    | PureHorzBox::Graphics { .. } => {
                        fire_inline_frame(interp, doc, i, line.x + *dx, line.baseline_y, bx)?;
                    }
                    PureHorzBox::InlineFrameMarker {
                        id,
                        end: false,
                        height,
                        depth,
                    } => {
                        open_inline.push(OpenInlineFrame {
                            id: *id,
                            x: line.x + *dx,
                            baseline_y: line.baseline_y,
                            height: *height,
                            depth: *depth,
                            carried: false,
                        });
                    }
                    PureHorzBox::InlineFrameMarker { id, end: true, .. } => {
                        // Innermost still-open frame with this id (well-nested
                        // by construction — `prim_inline_frame_breakable`
                        // always splices a matched pair around its own
                        // contents). Closing on the line it opened on is an
                        // UNBROKEN frame: `decoS`. Closing on a later line
                        // makes this the last of several fragments: `decoT`.
                        if let Some(pos) = open_inline.iter().rposition(|f| f.id == *id) {
                            let frame = open_inline.remove(pos);
                            let deco_idx = if frame.carried { 3 } else { 0 };
                            fire_inline_frame_fragment(
                                interp,
                                doc,
                                i,
                                &frame,
                                line.x + *dx,
                                deco_idx,
                            )?;
                        }
                    }
                    PureHorzBox::FrameMarker { id, end: false } => {
                        open.push(OpenFrame {
                            id: *id,
                            x: line.x + *dx,
                            marker_baseline: line.baseline_y,
                            top: None,
                            bottom: None,
                            open_seq: next_open_seq,
                            carried: false,
                        });
                        next_open_seq += 1;
                    }
                    PureHorzBox::FrameMarker { id, end: true } => {
                        // Close the innermost still-open frame with this id
                        // (well-nested by construction: `prim_block_frame_
                        // breakable` always emits a matched Start/End pair
                        // around its own contents). A frame carried over from
                        // an earlier page fires its TAIL fragment (`decoT`,
                        // bottom pad only); one that opened on this page fires
                        // the single-fragment `decoS` (both pads).
                        if let Some(pos) = open.iter().rposition(|f| f.id == *id) {
                            let frame = open.remove(pos);
                            // EVERY fragment carries the top pad, not just the
                            // first: `chop_page` re-applies a still-open
                            // frame's `paddingT` at the top of each
                            // continuation page (upstream `pageBreak.ml:322`),
                            // so the pad is real space on this page too and the
                            // rect has to cover it — upstream's
                            // `handlePdf.ml:325-330` spans the rect from the
                            // fragment's `ypos`, above the `-% paddingT` shift
                            // it lays the contents out at.
                            let deco_idx = if frame.carried { 3 } else { 0 };
                            let incl_top = true;
                            let gr = fire_block_frame_fragment(
                                interp, doc, &frame, deco_idx, incl_top, true,
                            )?;
                            closings.push((frame.open_seq, gr));
                        }
                        // An End with no matching open frame can't happen given
                        // well-nested markers; ignored if it did.
                    }
                    PureHorzBox::EmbeddedBlock {
                        block, anchor_last, ..
                    } => {
                        // A `block-frame-breakable` can also hide INSIDE an
                        // `embed-block-breakable` (figbox's inline
                        // `\fig-on-right`/`\fig-on-left`, which draw their image
                        // from the frame's deco): its `FrameStart`/`FrameEnd`
                        // markers live in this atomic box's own placed lines, not
                        // the page flow, so the walk above never sees them. Fire
                        // those nested decos with absolute coordinates.
                        fire_embedded_block_frames(
                            interp,
                            doc,
                            i,
                            line.x + *dx,
                            line.baseline_y,
                            block,
                            *anchor_last,
                            &mut next_open_seq,
                            &mut closings,
                        )?;
                    }
                    _ => {}
                }
            }
            // Every REAL line (one with non-marker content) extends every
            // currently-open frame's (top, bottom) — pad Skips don't create
            // a `PlacedLine` at all, so they're naturally excluded here; the
            // ±pad compensation happens once, at close time, above.
            //
            // BODY lines only (`Page::body_lines`). The header and footer are
            // appended after the columns, and a frame carried across a page
            // boundary is open for this whole walk, so counting them stretched
            // every such frame's fragment from the header baseline to the
            // footer — easytable's `+code` blocks painted their grey background
            // over entire pages (4, 11, 12) instead of over their own lines.
            if line_idx < page.body_lines {
                if let Some((height, depth)) = placed_line_extent(line) {
                    let top = line.baseline_y - height;
                    let bottom = line.baseline_y + depth;
                    for f in &mut open {
                        f.top = Some(f.top.map_or(top, |t| t.min(top)));
                        f.bottom = Some(f.bottom.map_or(bottom, |b| b.max(bottom)));
                    }
                }
            }
            // An inline frame still open at the end of a line really did split
            // (upstream `append_framed_lines`' non-final `PureLine` arms): fire
            // this line's fragment — `decoH` for the first, `decoM` for every
            // later one. The frame stays open; the re-anchor at the top of the
            // next body line's turn moves it on.
            if is_body && !open_inline.is_empty() {
                let right = placed_line_right_edge(line);
                let pending: Vec<OpenInlineFrame> = open_inline.clone();
                for frame in &pending {
                    let deco_idx = if frame.carried { 2 } else { 1 };
                    fire_inline_frame_fragment(interp, doc, i, frame, right, deco_idx)?;
                }
                for f in &mut open_inline {
                    f.carried = true;
                }
            }
        }
        // Frames still open at page end straddle the following page break: fire
        // this page's fragment — a HEAD (`decoH`, top pad only) the first time a
        // frame spans, a MIDDLE (`decoM`, no pads) on every later page — and
        // keep the frame open so its remaining fragments (and eventual `decoT`)
        // fire on the pages ahead. A frame that never accumulated a real line
        // on this page (top/bottom still `None`) contributes nothing and does
        // NOT advance its fragment state: it stays `carried` as it was, so a
        // frame that opened at the very bottom of a page (no room for a line)
        // still fires its HEAD (or, if it also closes with content on a single
        // later page, a `decoS`) on the first page that actually holds its
        // content. Collect fires first (can't hold `&open` across the `&mut
        // interp` deco call), then mark exactly the frames that fired.
        let mut page_end_fires: Vec<(usize, Vec<GraphicsElem>)> = Vec::new();
        let mut fired_seqs: Vec<usize> = Vec::new();
        for frame in &open {
            if frame.top.is_none() && frame.bottom.is_none() {
                continue;
            }
            // Top pad on every fragment — see the `FrameMarker { end: true }`
            // arm above for why a carried fragment has one too.
            let deco_idx = if frame.carried { 2 } else { 1 };
            let gr = fire_block_frame_fragment(interp, doc, frame, deco_idx, true, false)?;
            page_end_fires.push((frame.open_seq, gr));
            fired_seqs.push(frame.open_seq);
        }
        for f in &mut open {
            if fired_seqs.contains(&f.open_seq) {
                f.carried = true;
            }
        }
        closings.extend(page_end_fires);

        closings.sort_by_key(|(seq, _)| *seq);
        for (_, gr) in closings {
            interp.page_graphics[i].extend(gr);
        }
    }
    interp.current_page = None;
    Ok(())
}

/// Fire one placed `block-frame-breakable` fragment's decoration (`decoS`/
/// `decoH`/`decoM`/`decoT` picked by `deco_idx` — 0/1/2/3, evalUtil.ml:169
/// `get_decoset` order) with its final geometry, returning the graphics it
/// draws (absolute page coordinates). `incl_top_pad`/`incl_bot_pad` select
/// which of the frame's `pads.t`/`pads.b` this fragment carries: the single
/// (`decoS`) fragment carries both, the head only the top, the tail only the
/// bottom, and a middle neither — matching `pageBreak.ml`'s per-fragment
/// padding. The rect spans the frame's accumulated (top, bottom) extent on the
/// current page; an empty frame (no real line between Start/End) falls back to
/// the Start marker's own baseline for a degenerate zero-height rect.
fn fire_block_frame_fragment(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    frame: &OpenFrame,
    deco_idx: usize,
    incl_top_pad: bool,
    incl_bot_pad: bool,
) -> Result<Vec<GraphicsElem>, eval::EvalError> {
    let (pads, width, deco, deco_version) = match &interp.decos[frame.id.0] {
        eval::DecoEntry::Block {
            pads,
            width,
            decoset,
            version,
        } => (*pads, *width, decoset[deco_idx].clone(), *version),
        eval::DecoEntry::Inline { .. } | eval::DecoEntry::InlineBreakable { .. } => {
            return eval::eval_error("BUG: inline deco behind a block-frame marker")
        }
    };
    let top = frame.top.unwrap_or(frame.marker_baseline);
    let bottom = frame.bottom.unwrap_or(frame.marker_baseline);
    let frame_top = if incl_top_pad { top - pads.t } else { top };
    let frame_bottom = if incl_bot_pad {
        bottom + pads.b
    } else {
        bottom
    };
    let pt = (frame.x, doc.geometry.paper_height - frame_bottom);
    // Record which DecoId is firing so a `register-destination` call
    // inside the deco (annot.satyh's `register-location-frame`) can tag
    // itself with it — see `Interp::current_deco_id`'s doc comment.
    interp.current_deco_id = Some(frame.id);
    let gr = primitives::apply_deco(
        interp,
        deco_version,
        deco,
        pt,
        width,
        frame_bottom - frame_top,
        Length::ZERO,
    )?;
    interp.current_deco_id = None;
    Ok(gr)
}

/// Fire block-frame decorations that live INSIDE an `EmbeddedBlock` inline box.
///
/// figbox's inline `\fig-on-right`/`\fig-on-left` wrap a `block-frame-breakable`
/// (whose deco draws the figure image) in `embed-block-breakable`, so the
/// frame's `FrameStart`/`FrameEnd` markers end up in the embedded block's OWN
/// placed lines, never the page flow that [`fire_hooks`] walks — the image
/// would silently never render. Replicate `place_embedded_block`'s transform
/// (`rustyfi-pdf`): `place_block_at` from a zero origin, then shift so the
/// anchor line (first for top-anchor, last for bottom) sits at the box's inline
/// baseline. Converting that writer y-up geometry back to page y-down, inner
/// line `i`'s absolute baseline is `baseline_ydown + (bl_i - anchor_offset)` and
/// its x is `tx + line.x + dx`. Over those absolute lines we run the same
/// frame-open/close tracking and `fire_block_frame_fragment` as the main walk.
/// The box is atomic (one inline box, not page-broken), so every nested frame
/// opens and closes within it and fires a single-fragment `decoS`. Nested
/// `EmbeddedBlock`s (a figbox inside a figbox) recurse.
#[allow(clippy::too_many_arguments)]
fn fire_embedded_block_frames(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    page: usize,
    tx: Length,
    baseline_ydown: Length,
    block: &[VertBox],
    anchor_last: bool,
    next_open_seq: &mut usize,
    out: &mut Vec<(usize, Vec<GraphicsElem>)>,
) -> Result<(), eval::EvalError> {
    let placed = place_block_at((Length::ZERO, Length::ZERO), block.to_vec());
    let anchor = if anchor_last {
        placed.last()
    } else {
        placed.first()
    };
    let Some(anchor) = anchor else {
        return Ok(());
    };
    let anchor_offset = anchor.baseline_y;
    let mut open: Vec<OpenFrame> = Vec::new();
    // `inline-frame-breakable` inside an embedded block, same story as the
    // `PureHorzBox::Frame` arm below: latexcmds' `\fbox`/`\doublebox`/
    // `\ovalbox`/`\shadowbox` all go through the BREAKABLE primitive, and a
    // `+listing` item's lines live here rather than in the page flow.
    let mut open_inline: Vec<OpenInlineFrame> = Vec::new();
    for pl in &placed {
        let abs_baseline = baseline_ydown + (pl.baseline_y - anchor_offset);
        for f in &mut open_inline {
            f.x = tx + pl.x;
            f.baseline_y = abs_baseline;
        }
        for (dx, bx) in &pl.contents {
            match bx {
                PureHorzBox::InlineFrameMarker {
                    id,
                    end: false,
                    height,
                    depth,
                } => {
                    open_inline.push(OpenInlineFrame {
                        id: *id,
                        x: tx + pl.x + *dx,
                        baseline_y: abs_baseline,
                        height: *height,
                        depth: *depth,
                        carried: false,
                    });
                }
                PureHorzBox::InlineFrameMarker { id, end: true, .. } => {
                    if let Some(pos) = open_inline.iter().rposition(|f| f.id == *id) {
                        let frame = open_inline.remove(pos);
                        let deco_idx = if frame.carried { 3 } else { 0 };
                        fire_inline_frame_fragment(
                            interp,
                            doc,
                            page,
                            &frame,
                            tx + pl.x + *dx,
                            deco_idx,
                        )?;
                    }
                }
                PureHorzBox::FrameMarker { id, end: false } => {
                    open.push(OpenFrame {
                        id: *id,
                        x: tx + pl.x + *dx,
                        marker_baseline: abs_baseline,
                        top: None,
                        bottom: None,
                        open_seq: *next_open_seq,
                        carried: false,
                    });
                    *next_open_seq += 1;
                }
                PureHorzBox::FrameMarker { id, end: true } => {
                    if let Some(pos) = open.iter().rposition(|f| f.id == *id) {
                        let frame = open.remove(pos);
                        let gr = fire_block_frame_fragment(interp, doc, &frame, 0, true, true)?;
                        out.push((frame.open_seq, gr));
                    }
                }
                // An INLINE frame (`inline-frame-outer`/`-inner`/`-breakable`)
                // can hide in here too — latexcmds' `\fbox`/`\doublebox`/
                // `\ovalbox`/`\shadowbox` used inside `+listing` items, whose
                // lines live in an embedded block rather than the page flow
                // (26 of 144 inline frames in one document went undrawn
                // without this). `Tabular`/`Graphics` too — see the identical
                // arm in `fire_hooks`.
                PureHorzBox::Frame { .. }
                | PureHorzBox::Tabular(_)
                | PureHorzBox::Graphics { .. } => {
                    fire_inline_frame(interp, doc, page, tx + pl.x + *dx, abs_baseline, bx)?;
                }
                PureHorzBox::EmbeddedBlock {
                    block: inner,
                    anchor_last: al,
                    ..
                } => {
                    fire_embedded_block_frames(
                        interp,
                        doc,
                        page,
                        tx + pl.x + *dx,
                        abs_baseline,
                        inner,
                        *al,
                        next_open_seq,
                        out,
                    )?;
                }
                _ => {}
            }
        }
        if let Some((height, depth)) = placed_line_extent(pl) {
            let top = abs_baseline - height;
            let bottom = abs_baseline + depth;
            for f in &mut open {
                f.top = Some(f.top.map_or(top, |t| t.min(top)));
                f.bottom = Some(f.bottom.map_or(bottom, |b| b.max(bottom)));
            }
        }
        if !open_inline.is_empty() {
            let right = tx + placed_line_right_edge(pl);
            let pending: Vec<OpenInlineFrame> = open_inline.clone();
            for frame in &pending {
                let deco_idx = if frame.carried { 2 } else { 1 };
                fire_inline_frame_fragment(interp, doc, page, frame, right, deco_idx)?;
            }
            for f in &mut open_inline {
                f.carried = true;
            }
        }
    }
    Ok(())
}

/// Apply one `hook-page-break` closure to `(pbinfo, point)`.
///
/// Extracted so the walk can fire a hook wherever it is found, not only at
/// the top level of a placed line: `stdja`'s `+section` appends its
/// `hook-page-break` to the heading's inline boxes, wrapped in the title
/// deco's inline FRAME, so all 7 of this manual's hooks sat one level down
/// and none fired without this. `stdja.satyh:448` registers `<label>:page`
/// from inside this closure, so an unfired hook left `get-cross-reference`
/// rendering `?`: 11 such `?` for easytable and 21 for enumitem on pages
/// 1-2 alone, where SATySFi emits none.
fn fire_page_break_hook(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    page_number: i64,
    x: Length,
    baseline_y: Length,
    id: rustyfi_backend::HookId,
) -> Result<(), eval::EvalError> {
    let closure = interp.hooks[id.0].clone();
    let mut fields = BTreeMap::new();
    fields.insert("page-number".to_string(), Value::Int(page_number));
    let pbinfo = Value::Record(fields);
    // PDF page space is y-up; placed geometry (`baseline_y`) is page space
    // y-down from the paper top — the same flip the writers apply.
    let point = Value::Tuple(vec![
        Value::Length(x),
        Value::Length(doc.geometry.paper_height - baseline_y),
    ]);
    let applied = interp.apply(closure, pbinfo)?;
    match interp.apply(applied, point)? {
        Value::Unit => Ok(()),
        other => eval::eval_error(format!(
            "hook-page-break closure returned {}, expected unit",
            other.type_name()
        )),
    }
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
///
/// Also the entry point for firing whatever is nested INSIDE a placed box:
/// besides an inline frame's own contents this descends into a
/// `Tabular`'s cells, since a cell's boxes never reach the page flow that
/// [`fire_hooks`] walks. Everything a cell can carry — a `\href`, a `\ref`,
/// a `hook-page-break` — was silently inert before: a `\href` in an
/// easytable cell produced no `/Link` annotation at all, which is 3 of
/// slydifi's 4 links. `bx` that is neither a frame nor a tabular is a no-op,
/// so callers can hand it every box on a line.
fn fire_inline_frame(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    page: usize,
    x: Length,
    baseline_y: Length,
    bx: &PureHorzBox,
) -> Result<(), eval::EvalError> {
    let contents = match bx {
        PureHorzBox::Frame {
            width,
            height,
            depth,
            deco,
            contents,
        } => {
            let (deco_v, deco_version) = match &interp.decos[deco.0] {
                eval::DecoEntry::Inline { deco, version } => (deco.clone(), *version),
                eval::DecoEntry::Block { .. } | eval::DecoEntry::InlineBreakable { .. } => {
                    return eval::eval_error("BUG: block deco behind an inline frame")
                }
            };
            let pt = (x, doc.geometry.paper_height - baseline_y);
            // See the block-frame call site's identical comment —
            // `annot.satyh`'s `\href` fires `register-link-to-uri` from exactly
            // this closure.
            interp.current_deco_id = Some(*deco);
            let gr =
                primitives::apply_deco(interp, deco_version, deco_v, pt, *width, *height, *depth)?;
            interp.current_deco_id = None;
            interp.page_graphics[page].extend(gr);
            contents
        }
        PureHorzBox::Tabular(tab) => {
            // Each cell is its own placed run on its own baseline. The
            // writers' convention (`rustyfi-pdf`'s `emit_box`, `ty +
            // cell.baseline_y` in PDF y-UP space) means `cell.baseline_y` is
            // measured upward from the tabular box's own baseline, so in this
            // walk's y-DOWN page coordinates it subtracts.
            for cell in &tab.cells {
                fire_nested_in_contents(
                    interp,
                    doc,
                    page,
                    x + cell.x,
                    baseline_y - cell.baseline_y,
                    &cell.contents,
                )?;
            }
            return Ok(());
        }
        PureHorzBox::Graphics {
            elems,
            origin_independent,
            ..
        } => {
            // `draw-text` runs (`GraphicsElem::Text`) carry real inline boxes,
            // and figbox's `textbox` puts whole tables in one — slydifi's
            // theme table reaches its `\link`s only through here. Element
            // coordinates are box-local PDF y-up from the box's placed anchor,
            // except for an `origin_independent` box whose callback already
            // produced page-absolute ones: exactly `rustyfi-pdf`'s own
            // `(ax, ay)` choice, kept in step with it.
            let anchor_y = if *origin_independent {
                doc.geometry.paper_height
            } else {
                baseline_y
            };
            let anchor_x = if *origin_independent { Length::ZERO } else { x };
            fire_nested_in_graphics(interp, doc, page, anchor_x, anchor_y, elems)?;
            return Ok(());
        }
        _ => return Ok(()),
    };
    fire_nested_in_contents(interp, doc, page, x, baseline_y, contents)
}

/// Fire every hook, decoration and deferred destination carried by a graphics
/// box's elements — the inline runs a `draw-text` (`GraphicsElem::Text`)
/// holds, and the `GraphicsElem::Destination` markers an `inline-graphics`
/// callback's `register-destination` calls left behind — recursing through
/// `Group`/`Clip`. `anchor_x`/`anchor_y` are the box's placed origin in this
/// walk's (x, y-DOWN) page coordinates.
///
/// A `Text` element's own `transform` (from `rotate-graphics`/
/// `scale-graphics`) is deliberately NOT applied: a decoration's rect is an
/// axis-aligned rectangle, so a rotated run has no faithful rect to report,
/// and firing at the untransformed anchor at least puts a `\href`'s link
/// where the run starts rather than nowhere at all.
fn fire_nested_in_graphics(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    page: usize,
    anchor_x: Length,
    anchor_y: Length,
    elems: &[GraphicsElem],
) -> Result<(), eval::EvalError> {
    for elem in elems {
        match elem {
            GraphicsElem::Text { pt, contents, .. } => {
                // `pt` is PDF y-UP relative to the anchor; this walk is y-down.
                fire_nested_in_contents(
                    interp,
                    doc,
                    page,
                    anchor_x + pt.0,
                    anchor_y - pt.1,
                    contents,
                )?;
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
                fire_nested_in_graphics(interp, doc, page, anchor_x, anchor_y, inner)?;
            }
            // A `register-destination` made from an eagerly-applied
            // `inline-graphics` callback, replayed now that the box HAS a page
            // and a placed point — the whole reason the eager shortcut does
            // not simply lose it. `pt` is PDF y-UP from the anchor, as a
            // `Text`'s is, and a `NamedDest`'s `y` is y-up too, so converting
            // the anchor back out of this walk's y-down frame is the only
            // arithmetic needed. An `origin_independent` box arrives here with
            // `anchor_y = paper_height`/`anchor_x = 0` (see the caller), which
            // makes this the identity — exactly the page-absolute reading its
            // ink gets from the writers.
            GraphicsElem::Destination { key, pt } => {
                let name = interp.dest_name(key);
                let x = anchor_x + pt.0;
                let y = doc.geometry.paper_height - anchor_y + pt.1;
                // Same `current_deco_id` tagging as a direct call: an
                // `inline-graphics` anchor can sit inside a firing deco's
                // contents, and the reflow backend resolves a destination to
                // its Frame through this id.
                if let Some(deco_id) = interp.current_deco_id {
                    interp.dest_decos.push((deco_id, name.clone()));
                }
                interp.destinations.push(rustyfi_backend::NamedDest {
                    page,
                    name,
                    x,
                    y,
                });
            }
            GraphicsElem::Fill(..) | GraphicsElem::Stroke(..) | GraphicsElem::DashedStroke(..) => {}
        }
    }
    Ok(())
}

/// Fire every hook and decoration inside one placed content run — an inline
/// frame's contents or a tabular cell's — with `x0`/`baseline_y` as the run's
/// own absolute origin.
///
/// An `inline-frame-breakable` reached through here is spliced into the run as
/// a marker pair (see `prim_inline_frame_breakable`). Such a run is a single
/// `fit_cell` line, so the frame is always unbroken and always fires `decoS` —
/// which is also what upstream does in this position, since a breakable frame
/// reached through a *pure* box degrades to an atomic `LBOuterFrame`
/// (`convert_list_for_line_breaking_pure`, lineBreak.ml:335).
fn fire_nested_in_contents(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    page: usize,
    x0: Length,
    baseline_y: Length,
    contents: &[(Length, PureHorzBox)],
) -> Result<(), eval::EvalError> {
    let mut open_inline: Vec<OpenInlineFrame> = Vec::new();
    for (dx, child) in contents {
        // A `hook-page-break` can sit INSIDE the frame — `stdja`'s `+section`
        // appends one to a heading that the title deco then wraps in a frame —
        // and the top-level walk never sees it. See `fire_page_break_hook`.
        if let PureHorzBox::HookPageBreak { id } = child {
            fire_page_break_hook(interp, doc, (page + 1) as i64, x0 + *dx, baseline_y, *id)?;
        }
        match child {
            PureHorzBox::InlineFrameMarker {
                id,
                end: false,
                height,
                depth,
            } => open_inline.push(OpenInlineFrame {
                id: *id,
                x: x0 + *dx,
                baseline_y,
                height: *height,
                depth: *depth,
                carried: false,
            }),
            PureHorzBox::InlineFrameMarker { id, end: true, .. } => {
                if let Some(pos) = open_inline.iter().rposition(|f| f.id == *id) {
                    let frame = open_inline.remove(pos);
                    fire_inline_frame_fragment(interp, doc, page, &frame, x0 + *dx, 0)?;
                }
            }
            _ => {}
        }
        fire_inline_frame(interp, doc, page, x0 + *dx, baseline_y, child)?;
    }
    Ok(())
}
