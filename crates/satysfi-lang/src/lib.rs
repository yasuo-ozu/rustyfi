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
pub mod value;

use crossref::{CrossRefs, Verdict};
use satysfi_backend::{FontMetrics, PureHorzBox};
use std::cell::RefCell;
use std::collections::BTreeMap;
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
        let env = primitives::base_env();
        let mut interp = eval::Interp::new(metrics);
        interp.crossrefs = crossrefs.clone();
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
            Verdict::CanTerminate(_) | Verdict::CountMax => return Ok((doc, trials)),
        }
    }
}

/// Fire every placed page-break hook, in document order, now that final page
/// numbers and points are known. THIS is the port's `make_hook` +
/// `handlePdf.ml:234/337`'s invocation, relocated to the one place that
/// legally holds `&mut Interp` — the backend produced the geometry (a POD
/// `HookId` riding inside `PureHorzBox::HookPageBreak`, per `hbox.rs`); this
/// reads it back and re-enters the evaluator.
/// docs/plans/hooks-annotations-crossref.md §The callback architecture.
///
/// `pub` (rather than crate-private) so unit tests can drive it directly
/// against a hand-built `DocumentValue`, without going through a full
/// `compile_document_cst` fixpoint.
pub fn fire_hooks(interp: &mut eval::Interp, doc: &DocumentValue) -> Result<(), eval::EvalError> {
    for (i, page) in doc.pages.iter().enumerate() {
        let page_number = (i + 1) as i64; // 1-based, = pbinfo#page-number
        for line in &page.lines {
            for (dx, bx) in &line.contents {
                let PureHorzBox::HookPageBreak { id } = bx else {
                    continue;
                };
                let closure = interp.hooks[id.0].clone();
                let mut fields = BTreeMap::new();
                fields.insert("page-number".to_string(), Value::Int(page_number));
                let pbinfo = Value::Record(fields);
                // PDF page space is y-up; placed geometry (`baseline_y`) is
                // page space y-down from the paper top — the same flip the
                // writers apply (e.g. `satysfi-pdf/src/lib.rs`'s `paper_h -
                // baseline_y`). Slice 1's fixture never reads the point, but
                // a future annotation consumer (roadmap §B) needs it correct.
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
        }
    }
    Ok(())
}
