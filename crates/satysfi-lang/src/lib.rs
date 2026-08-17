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
use satysfi_backend::{placed_line_extent, DecoId, FontMetrics, GraphicsElem, Length, PureHorzBox};
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
                };
                return Ok((Rc::new(final_doc), trials));
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
