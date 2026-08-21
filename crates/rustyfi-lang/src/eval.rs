//! Interpreter state and beta-reduction.
//!
//! This was the tree-walking evaluator (the naive-interpreter shape of
//! evaluator.cppo.ml; the bytecode VM was intentionally not ported). Phases 3
//! and 4 of retired the tree-walk itself — expression evaluation lives
//! entirely in [`crate::compile`] now — leaving here what is genuinely
//! runtime: the [`Interp`] state every primitive threads (images, hooks,
//! cross-references, decorations, …), function application, and pattern
//! matching.

use crate::ast::{Ast, Pattern};
use crate::crossref::CrossRefs;
use crate::value::{BaseEnv, Env, Value};
use rustyfi_backend::{DocInfo, FontMetrics, ImageResource, MathCmdId};
use rustyfi_syntax::{RustyfiVersion, Span};
use std::cell::RefCell;
use std::rc::Rc;

/// See [`Interp::decos`].
///
/// Each entry records the `interp.version` that was active when the deco
/// closure was CAPTURED ([`DecoEntry::version`]). This used to be the X2b
/// audit's known residual gap: the entry carried no version, and its
/// consumer (`primitives::apply_deco`, called only from `lib.rs`'s
/// post-page-break hook-firing pass) read `interp.version` at FIRE time
/// instead. Fire time is always outside every `VersionScope`'s save/restore
/// window, so in a cross-version program the flag there is the ENTRY's
/// generation, never the deco author's.
///
/// The corpus makes that concrete: `uline`, `enumitem` and `figbox` are
/// ordinary 0.0.6 packages that call `inline-frame-*`/`block-frame-*`
/// themselves, with their own 0.0.6 `graphics list` decos. Nothing about
/// them is an export-boundary `deco` (X3b's business); they just register a
/// deco while `interp.version` is `V0_0` and have it fired while it is
/// `V0_1`, so `coerce_graphics_result` demanded a single `graphics` and got
/// a list — "expected graphics, got list", at eval time, from inside a
/// package the user never wrote. Capturing the version at push time is
/// exactly the promise the splice already makes for every other forked
/// primitive (`Ast::VersionScope`'s eval arm), just extended to the one
/// consumer that runs after the window closes.
#[derive(Clone, Debug)]
pub enum DecoEntry {
    Inline {
        deco: Value,
        /// The generation whose `deco` calling convention this closure obeys
        /// — see this enum's doc comment.
        version: RustyfiVersion,
    },
    Block {
        pads: rustyfi_backend::Paddings,
        /// The frame's OUTER width (the wrapping context's paragraph_width).
        width: rustyfi_backend::Length,
        /// `(decoS, decoH, decoM, decoT)` — evalUtil.ml:169 `get_decoset`.
        decoset: [Value; 4],
        /// The generation whose `deco` calling convention these closures
        /// obey — see this enum's doc comment.
        version: RustyfiVersion,
    },
    /// `inline-frame-breakable`'s deco set, behind a
    /// `PureHorzBox::InlineFrameMarker` pair. The inline twin of `Block`
    /// above: the frame may split across LINE breaks rather than page breaks,
    /// so `fire_hooks` picks `decoS`/`decoH`/`decoM`/`decoT` per line
    /// fragment the same way. `pads` is kept for the vertical half only —
    /// `paddingL`/`paddingR` are already spliced into the box stream as
    /// `FixedEmpty` (upstream `append_horz_padding`), so only `t`/`b` are
    /// read back here, to size each fragment's rect.
    InlineBreakable {
        pads: rustyfi_backend::Paddings,
        /// `(decoS, decoH, decoM, decoT)` — evalUtil.ml:169 `get_decoset`.
        decoset: [Value; 4],
        /// The generation whose `deco` calling convention these closures
        /// obey — see this enum's doc comment.
        version: RustyfiVersion,
    },
}

impl DecoEntry {
    /// The generation this entry's deco closure(s) were captured under — the
    /// one `primitives::apply_deco` must decode their result with, rather
    /// than whatever `interp.version` happens to be at fire time.
    pub fn version(&self) -> RustyfiVersion {
        match self {
            DecoEntry::Inline { version, .. }
            | DecoEntry::Block { version, .. }
            | DecoEntry::InlineBreakable { version, .. } => *version,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{}{msg}", .span.map(|s| format!("{s}: ")).unwrap_or_default())]
pub struct EvalError {
    pub span: Option<Span>,
    pub msg: String,
}

pub fn eval_error<T>(msg: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError {
        span: None,
        msg: msg.into(),
    })
}

/// Comma-separated, sorted field names of a record — the "(available fields:
/// …)" hint shared by the field-access and field-update error messages.
pub(crate) fn available_fields(map: &std::collections::BTreeMap<String, Value>) -> String {
    let mut keys: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
    keys.sort();
    keys.join(", ")
}

/// Evaluation state: the font-metrics seam (and later: cross references,
/// mutable stores).
pub struct Interp<'a> {
    pub metrics: &'a dyn FontMetrics,
    /// The document-wide image table: `load-image`
    /// (`primitives::prim_load_image`) decodes eagerly and pushes here,
    /// returning the new entry's index as `Value::Image`;
    /// `use-image-by-width` (`primitives::prim_use_image_by_width`) looks
    /// the resource back up by that index. `page-break`
    /// (`primitives::prim_page_break`) clones this out into
    /// `DocumentValue::images` when it packages the final document, so the
    /// PDF writer sees every image ever decoded while evaluating (a superset
    /// of what actually ends up placed on a page — the writer itself filters
    /// down to the ones a placed line actually references).
    pub images: Vec<ImageResource>,
    /// The document-wide page-break-hook closure table: `hook-page-break`
    /// pushes its closure argument here and returns a `HookId` index (via
    /// `PureHorzBox::HookPageBreak`) — the same `ImageId`-style seam as
    /// `images` above, but for a deferred *computation* rather than a resource.
    /// Reset every trial (see `crossrefs`, which is the one exception), read
    /// back by `fire_hooks` once `break_pages` has placed every hook and its
    /// final geometry is known.
    pub hooks: Vec<Value>,
    /// Installed-math-command table (`get-initial-context`/
    /// `set-math-command` push here; `Context::math_command` holds the
    /// index) — the `ImageId`/`HookId`-style seam, because the backend
    /// `Context` cannot hold a lang-side `Value`. Read back by
    /// `read_inline`'s `EmbedMath` arm.
    pub math_commands: Vec<Value>,
    /// The cross-reference table, shared with the compile driver
    /// (`lib.rs::compile_document_cst`) across every trial of the fixpoint
    /// loop — unlike `hooks`/`images`, this must *not* reset per trial, so
    /// the driver constructs one `Rc<RefCell<CrossRefs>>` and clones the
    /// handle into each trial's fresh `Interp`. Defaults to a fresh empty
    /// table so existing single-run call sites/unit tests compile unchanged.
    pub crossrefs: Rc<RefCell<CrossRefs>>,
    /// §B/§C accumulators: link annotations / named destinations /
    /// outline entries, plus the per-page deco-graphics overlays (§D).
    /// All reset per trial (fresh `Interp`); the FINAL trial's contents
    /// are moved into `DocumentValue::extras` by
    /// `compile_document_cst_with_trials`.
    pub annotations: Vec<rustyfi_backend::Annot>,
    pub destinations: Vec<rustyfi_backend::NamedDest>,
    pub outline: Vec<rustyfi_backend::OutlineEntry>,
    pub page_graphics: Vec<Vec<rustyfi_backend::GraphicsElem>>,
    /// `register-document-information`'s accumulator (prim-retype-sweep
    /// §2.4) — LAST WRITE WINS, same reset-per-trial policy as
    /// `outline`/`annotations`/`destinations` above (a fresh `Interp` per
    /// trial resets this to `None`; the final trial's value is drained into
    /// `DocExtras::doc_info` by `lib.rs`'s `eval_document_trials`).
    pub doc_info: Option<DocInfo>,
    /// `Some(0-based page)` only while `fire_hooks` is walking that page —
    /// the port of upstream's `State.during_page_break` + "current page"
    /// (`annotation.ml:15`, `namedDest.ml`'s `notify_pagebreak`).
    pub current_page: Option<usize>,
    /// S2 ("Links/metadata"): the `DecoId` of the deco closure currently
    /// being fired by `fire_hooks`' two `apply_deco` call sites (`lib.rs`),
    /// `None` outside any such window (e.g. inside a plain
    /// `hook-page-break` closure). This is the STRUCTURAL link between a
    /// placed `Annot`/`NamedDest` (page-absolute, known only
    /// post-page-break) and the `PureHorzBox::Frame`/
    /// `VertBox::FrameStart`/`FrameEnd` marker that produced it in the
    /// PRE-page-break `DocumentValue::reflow_source` — both carry the SAME
    /// `DecoId`, so `register_link`/`prim_register_destination` recording
    /// it here (into `link_decos`/`dest_decos` below) lets the reflow
    /// backend resolve "which Frame is this link" exactly, not by
    /// geometry/position (which `reflow_source` doesn't have).
    pub current_deco_id: Option<rustyfi_backend::DecoId>,
    /// One `(DecoId, action)` per `register-link-to-uri`/`-to-location`
    /// call made while `current_deco_id` was `Some` — see that field's doc
    /// comment. Reset per trial, drained into `DocumentValue::reflow_links`
    /// by `eval_document_trials` alongside `extras`.
    pub link_decos: Vec<(rustyfi_backend::DecoId, rustyfi_backend::AnnotAction)>,
    /// Same idea as `link_decos`, for `register-destination`
    /// (`annot.satyh`'s `register-location-frame` idiom): `(DecoId, name)`.
    /// Drained into `DocumentValue::reflow_dests`.
    pub dest_decos: Vec<(rustyfi_backend::DecoId, String)>,
    /// `namedDest.ml`'s key -> "nameddest{N}" sanitizer table (`name_from_
    /// hash_table`): arbitrary user keys become stable PDF name strings,
    /// shared by register-destination / register-link-to-location /
    /// register-outline within one trial.
    dest_names: std::collections::HashMap<String, String>,
    /// §D deco-closure table (`DecoId` indexes here) — `hooks`' twin for
    /// decorations. `Inline` holds one `deco` closure
    /// (`point -> length -> length -> length -> graphics list`); `Block`
    /// holds a block frame's four-closure deco-set + the geometry the
    /// markers can't carry. Reset per trial.
    pub decos: Vec<DecoEntry>,
    /// Deferred `inline-graphics-outer` callbacks (`length -> point ->
    /// graphics list`), indexed by `GraphicsFnId` — the `hooks` pattern.
    /// Reset per trial like `hooks`/`images`.
    /// Each entry is the deferred callback PLUS the generation it was
    /// registered under, for the same reason [`DecoEntry`] carries one: the
    /// callback's RESULT shape (`graphics list` vs one `graphics`) is a
    /// property of the code that wrote it, and
    /// `primitives::resolve_outer_graphics_in_contents` runs long after,
    /// from a line-breaking post-pass with no version context of its own.
    pub outer_graphics: Vec<(Value, RustyfiVersion)>,
    /// The target language version this evaluation run is checking against
    /// (math-split spec §3.4) — consulted only by `read_inline`'s
    /// `IText::EmbedMath` FALLBACK arm (no installed math command; unit-test
    /// contexts only — the installed-command path is version-blind already).
    /// Default `V0_0`; `lib.rs`'s `eval_document_trials` (the shared tail
    /// both `compile_document_cst_with_trials` and
    /// `compile_document_v1_with_trials` fall into) sets this to the real
    /// target version on every `Interp` it constructs.
    pub version: RustyfiVersion,
}

impl<'a> Interp<'a> {
    pub fn new(metrics: &'a dyn FontMetrics) -> Self {
        Interp {
            metrics,
            images: Vec::new(),
            hooks: Vec::new(),
            math_commands: Vec::new(),
            crossrefs: Rc::new(RefCell::new(CrossRefs::new())),
            annotations: Vec::new(),
            destinations: Vec::new(),
            outline: Vec::new(),
            page_graphics: Vec::new(),
            doc_info: None,
            current_page: None,
            current_deco_id: None,
            link_decos: Vec::new(),
            dest_decos: Vec::new(),
            dest_names: std::collections::HashMap::new(),
            decos: Vec::new(),
            outer_graphics: Vec::new(),
            version: RustyfiVersion::V0_0,
        }
    }

    /// Evaluate `ast` by compiling it against `env` and running the result.
    ///
    /// This used to be the reference **tree-walking** interpreter, kept beside
    /// [`crate::compile`]'s closure compiler so the two could be cross-checked.
    /// Phase 3 of retired it: quoted text is now compiled eagerly into
    /// [`crate::quoted`]'s name-free form, so a tree-walker can no longer build
    /// a `Value::InlineText` at all without invoking the compiler — there is
    /// exactly one evaluator now.
    ///
    /// Kept as this thin shim because ~25 integration tests drive the
    /// evaluator through it. It is precisely what those tests' compiled
    /// counterpart already did, which is why the differential harness that
    /// used to compare the two paths is gone: there is nothing left to differ.
    ///
    /// `base` is the COMPILE-time environment `ast`'s free names resolve
    /// against; the program itself runs in a fresh, empty runtime frame chain
    /// (Phase 4 — the base environment is no longer that chain's root,
    /// because nothing resolves a name at run time).
    pub fn eval(&mut self, base: &BaseEnv, ast: &Ast) -> Result<Value, EvalError> {
        crate::compile::compile_program(ast, base).run(&Env::root(), self)
    }

    /// Intern an installed math command, returning the handle a `Context`
    /// carries (`Context::math_command`).
    pub fn register_math_command(&mut self, cmd: Value) -> MathCmdId {
        self.math_commands.push(cmd);
        MathCmdId(self.math_commands.len() - 1)
    }

    /// `namedDest.ml:name_from_hash_table` — the stable PDF name for `key`,
    /// minting `nameddest{N}` on first sight. Also used by `register-outline`
    /// (upstream `Outline.make_entry` calls `NamedDest.get`, which mints too).
    pub fn dest_name(&mut self, key: &str) -> String {
        if let Some(n) = self.dest_names.get(key) {
            return n.clone();
        }
        let n = format!("nameddest{}", self.dest_names.len());
        self.dest_names.insert(key.to_string(), n.clone());
        n
    }

    pub fn apply(&mut self, func: Value, arg: Value) -> Result<Value, EvalError> {
        // A plain (0.0.6-shaped) application supplies no optional bundle; a
        // closure that *does* declare optional params (reached this way from
        // e.g. a higher-order caller) then defaults every one to `None`,
        // faithful to upstream's `reduce_beta_list`.
        self.apply_with_opts(func, Vec::new(), arg)
    }

    /// Beta-reduce `func` against a positional argument plus a SATySFi 0.1
    /// labeled-optional bundle. For a closure, each of the closure's declared
    /// optional params binds `Some v` when the bundle carries its label, else
    /// `None`; a supplied label the closure does not declare is ignored
    /// (upstream `reduce_beta` folds over the *closure's* map — the
    /// typechecker rejects genuinely-wrong labels first). This
    /// unknown-label-ignore is only sound because typecheck runs first.
    pub fn apply_with_opts(
        &mut self,
        func: Value,
        opt_vals: Vec<(String, Value)>,
        arg: Value,
    ) -> Result<Value, EvalError> {
        match func {
            Value::CompiledClosure {
                opt_labels,
                body,
                env,
            } => {
                // Slot order: the declared optional binders, then the
                // positional parameter — exactly what `Ast::LambdaOpt` pushed
                // onto the compiler's scope stack.
                let mut slots = Vec::with_capacity(opt_labels.len() + 1);
                push_opt_slots(&mut slots, &opt_labels, &opt_vals);
                slots.push(arg);
                body.run(&env.child(slots), self)
            }
            Value::Prim { def, mut applied } => {
                if !opt_vals.is_empty() {
                    return eval_error(
                        "labeled optional arguments to a primitive are roadmap phase 5",
                    );
                }
                applied.push(arg);
                if applied.len() == def.arity {
                    (def.run)(self, applied)
                } else {
                    Ok(Value::Prim { def, applied })
                }
            }
            other => eval_error(format!(
                "cannot apply a value of type {} as a function",
                other.type_name()
            )),
        }
    }
}

/// Append one slot per declared SATySFi 0.1 labeled-optional parameter, in
/// declaration order: `Some v` when `opt_vals` supplies that label, `None`
/// otherwise (upstream `reduce_beta`'s fold over the closure's own label map).
/// Supplied labels a closure does not declare are silently ignored — safe only
/// because typecheck rejects wrong labels first.
fn push_opt_slots(slots: &mut Vec<Value>, opt_labels: &[String], opt_vals: &[(String, Value)]) {
    for label in opt_labels {
        slots.push(match opt_vals.iter().find(|(l, _)| l == label) {
            Some((_, v)) => Value::Ctor("Some".to_string(), Some(Box::new(v.clone()))),
            None => Value::Ctor("None".to_string(), None),
        });
    }
}

/// Structural pattern matching against an already-evaluated scrutinee.
/// Returns `true` (and appends every bound value, POSITIONALLY, in the order
/// they were encountered) on a structural match; returns `false` (leaving
/// `bindings` for this attempt unusable — callers must use a fresh `Vec` per
/// arm) otherwise.
///
/// The push order here is the same left-to-right traversal
/// `compile::pattern_vars` uses to collect the arm's names, so position `i` in
/// `bindings` is slot `i` of the frame the arm runs in (Phase 4). Keep the two
/// in step. A pattern
/// and a value of mismatched shape is simply "no match", never an error:
/// this untyped evaluator relies on the (separate, not-yet-ported)
/// exhaustiveness/type checker to rule out ill-typed matches ahead of time.
pub fn match_pattern(pat: &Pattern, value: &Value, bindings: &mut Vec<Value>) -> bool {
    match pat {
        Pattern::Wild => true,
        Pattern::Var(_) => {
            bindings.push(value.clone());
            true
        }
        Pattern::As(inner_pat, _) => {
            if match_pattern(inner_pat, value, bindings) {
                bindings.push(value.clone());
                true
            } else {
                false
            }
        }
        Pattern::Unit => matches!(value, Value::Unit),
        Pattern::Bool(b) => matches!(value, Value::Bool(v) if v == b),
        Pattern::Int(n) => matches!(value, Value::Int(v) if v == n),
        Pattern::Str(s) => matches!(value, Value::Str(v) if v == s),
        Pattern::Tuple(ps) => match value {
            Value::Tuple(vs) if ps.len() == vs.len() => ps
                .iter()
                .zip(vs.iter())
                .all(|(p, v)| match_pattern(p, v, bindings)),
            _ => false,
        },
        Pattern::EmptyList => matches!(value, Value::List(vs) if vs.is_empty()),
        Pattern::Cons(head_pat, tail_pat) => match value {
            Value::List(vs) if !vs.is_empty() => {
                if !match_pattern(head_pat, &vs[0], bindings) {
                    return false;
                }
                let tail = Value::List(vs[1..].to_vec());
                match_pattern(tail_pat, &tail, bindings)
            }
            _ => false,
        },
        Pattern::Ctor(name, parg) => match value {
            Value::Ctor(vname, vpayload) if name == vname => match (parg, vpayload) {
                (None, None) => true,
                (Some(p), Some(v)) => match_pattern(p, v, bindings),
                _ => false,
            },
            _ => false,
        },
    }
}
