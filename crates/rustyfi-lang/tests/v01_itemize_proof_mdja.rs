//! Vendoring wave-4 Batch B: `itemize.satyh`/`proof.satyh`/`md-ja.satyh` —
//! three more real upstream SATySFi 0.1 packages, transliterated from
//! `saphe-split@b836d512` (see each package file's own header banner for
//! its exact upstream path + deltas) and PROVEN through the real
//! production loader (`rustyfi_loader::load`, `lib_root =
//! dist-v01/packages`, `RustyfiVersion::V0_1`) — not merely parsed.
//! Dependency order: `itemize` (no deps) -> `proof` (independent) ->
//! `md-ja` (LAST: `@require:`s `itemize`, transitively `math`/`code`/
//! `annot`/`hyph-english`/`unidata`/the 4 font stand-ins).
//!
//! Harness copied from `v01_stdlib.rs` (reproduced locally per that file's
//! own established convention — no shared test-support library target
//! exists in this crate): real loader -> per-file `lower_file_v1` prelude
//! concatenation -> `elaborate_program` -> `typecheck` -> `eval::Interp::
//! eval`, plus the same `assert_bare_access_unbound` qualified-export
//! negative probe every vendored package gets once (`v01_modules.rs`'s
//! `TopBinding::Module` wrapping makes every member reachable only as
//! `Pkg.member`).
//!
//! All three packages here export ONLY command bindings (`+cmd`/`\cmd`, or
//! `\cmd : math […]` for `proof`) — no plain top-level value. So instead
//! of asserting a bare `Value` straight off a package function (as
//! `v01_stdlib.rs` does for e.g. `Length.max`), the "value" probes below
//! build a real `context`, invoke the command through `read-block`/
//! `read-inline`/`read-math`, and extract a real computed `Length` via
//! `get-natural-length`/`Inline.get-natural-advance` (`itemize`/`proof`) —
//! or, for `md-ja` (whose `document` needs a full page-break to mean
//! anything), a real rendered `DocumentValue` via `compile_document_v1`,
//! mirroring `v01_stdlib.rs`'s own font/hyph-unidata capstone tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedFile};
use rustyfi_syntax::RustyfiVersion;

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist-v01/packages")
}

struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-v01-itemize-proof-mdja-{tag}-{}-{}.saty",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp fixture");
        TempDoc(path)
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// `FontMetrics` stub for the pure-computation ("value bar") tests below —
/// never actually consulted for glyph shaping in a meaningful way (the
/// `itemize`/`proof` value probes only measure natural lengths of ASCII-
/// free or near-empty content); mirrors `v01_stdlib.rs`'s `NoFonts`.
struct NoFonts;

impl FontMetrics for NoFonts {
    fn advance(&self, _f: FontKey, _c: char, _size: Length) -> Option<Length> {
        None
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size
    }
    fn descender(&self, _f: FontKey, _size: Length) -> Length {
        Length::pt(0.0)
    }
}

/// A real, FULLY-COVERING `FontMetrics`, for `md-ja`'s document capstone,
/// which DOES render real text through `MDJa.document`'s `read-inline`/
/// `read-block` passes. Unlike `v01_stdlib.rs`'s/`v01_slice1.rs`'s own
/// ASCII-only `Mono`, this stub advances EVERY character: `md-ja`'s
/// `document` renders a CJK back-matter heading (`参考文献`, "References")
/// whenever `\reference` has populated `reference-acc` — which this
/// capstone exercises — so an ASCII-only stub would fail glyph lookup on
/// `参`. `itemize`/`proof`'s own value probes reuse this same stub (they
/// only render ASCII, so the wider coverage is harmless there).
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn as_v01(f: &LoadedFile) -> &rustyfi_syntax::cst_v1::FileV1 {
    match &f.cst {
        LoadedCst::V0_1(cst) => cst,
        LoadedCst::V0_0_6(_) => unreachable!("this test's helper is V0_1-only"),
    }
}

/// Load `src` through the REAL multi-file loader, assemble the synthetic
/// `cst::File` exactly the way `compile_document_v1_with_trials` does
/// (`lib.rs:165-195`), then elaborate -> typecheck -> eval directly to a
/// `Value` (mirrors `v01_stdlib.rs`'s own `compile_v01_via_loader`).
fn compile_v01_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    compile_v01_via_loader_with_metrics(tag, src, &NoFonts)
}

fn compile_v01_via_loader_with_metrics(
    tag: &str,
    src: &str,
    metrics: &dyn FontMetrics,
) -> Result<Value, String> {
    let doc = TempDoc::new(tag, src);
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&doc.0, &opts).map_err(|e| format!("load: {e}"))?;

    let (entry, deps) = program
        .files
        .split_last()
        .expect("loader always yields at least the entry file");
    let mut prelude = Vec::new();
    for dep in deps {
        prelude.extend(lower::lower_file_v1(as_v01(dep)).map_err(|e| format!("lower dep: {e}"))?);
    }
    let entry_cst = as_v01(entry);
    let body = lower::lower_document_v1(entry_cst).map_err(|e| format!("lower entry: {e}"))?;
    let eoi = match entry_cst {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => return Err("entry must parse as a V0_1 document".to_string()),
    };
    let file = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(rustyfi_syntax::leaf::KwIn(rustyfi_syntax::Span::default())),
        body: Some(body),
        eoi,
    };

    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    // NOTE (vs `v01_stdlib.rs`'s harness, which uses the bare
    // `Scope::new`): the elaborate scope's version gates the `?(l = x)`
    // labeled-optional path (`elaborate.rs`'s `fun_rows_to_ast`/bundle
    // lowering rejects it under 0.0.6), and `Scope::new` defaults to
    // 0.0.6. The wave-0 stdlib packages never used a `?(…)` binder, so
    // `v01_stdlib.rs` never hit this; `itemize`/`proof`/`md-ja` all do
    // (def-site optional command bundles), so this harness must elaborate
    // under an explicit V0_1 scope.
    let scope = elaborate::Scope::new_with_version(env.names(), RustyfiVersion::V0_1);
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, RustyfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(metrics);
    // `coerce_graphics_result` (mirrors `v01_stdlib_graphics.rs`'s harness)
    // forks on `interp.version` at EVAL time (unlike primitive *selection*,
    // which `base_env_with_version` already resolved): a graphics callback
    // primitive (`inline-graphics`/`unite-graphics`/the deco family, all
    // fired for real by `itemize`'s bullets and `proof`'s inference bar)
    // expects the callback to return a single `graphics` under V0_1 but a
    // `list graphics` under the default V0_0_6 — so this must be set, or
    // those callbacks eval-error with "expected list, got graphics".
    interp.version = RustyfiVersion::V0_1;
    interp
        .eval(&env, &elaborated.body)
        .map_err(|e| format!("eval: {e}"))
}

/// The qualified-export negative probe every vendored package gets once
/// (see this module's doc comment): `bare_expr` referencing a package
/// member WITHOUT its `Pkg.` qualifier, after only `@require: <require>`,
/// must fail "unbound variable".
fn assert_bare_access_unbound(tag: &str, require: &str, bare_expr: &str) {
    let src = format!("@require: {require}\n{bare_expr}");
    let err = compile_v01_via_loader(tag, &src)
        .err()
        .unwrap_or_else(|| panic!("[{tag}] expected bare `{bare_expr}` to fail, it compiled"));
    assert!(
        err.contains("unbound variable"),
        "[{tag}] expected an unbound-variable error, got: {err}"
    );
}

fn as_length(v: Value) -> Length {
    match v {
        Value::Length(l) => l,
        other => panic!("expected a length, got {other:?}"),
    }
}

/// Run `f` on a thread with a generously large stack — every package in
/// this file transitively `@require:`s `list.satyg` (over 280 lines,
/// bigger than `stdlib_tier0.rs`'s own `gr.satyh` benchmark), which needs
/// more depth than the default stack allows through syan's recursive-
/// descent parser (mirrors `v01_stdlib.rs`'s own `run_with_big_stack`,
/// reproduced locally per this crate's established per-file-helper
/// convention).
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

// ============================================================================
// `itemize.satyh` — sealed; `+listing`/`\listing`/`+enumerate`/`\enumerate`
// are ALL commands (no plain value export), so the "value" probe below
// builds a `context` (via `get-initial-context`, needing only a command
// VALUE for the initial inline-math command slot — `V01Mini`'s own
// `\math`) and measures `+Itemize.listing`'s/`+Itemize.enumerate`'s
// rendered `block-boxes` through the bare global `get-natural-length`
// primitive (`primitives.rs`'s own doc comment on it: "`get-natural-
// width`'s block sibling").
// ============================================================================

#[test]
fn itemize_bare_listing_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("itemize-bare", "itemize", "command \\listing");
    });
}

#[test]
fn itemize_listing_and_enumerate_compute_positive_natural_lengths() {
    run_with_big_stack(|| {
        let src = "@require: v01-mini
@require: itemize

let open V01Mini in
let ctx = get-initial-context 400pt (command \\math) in
let len-listing =
  get-natural-length
    (read-block ctx '<+Itemize.listing?(break = true)(Item({one}, [Item({nested}, [])]));>)
in
let len-enumerate =
  get-natural-length
    (read-block ctx '<+Itemize.enumerate(Item({}, [Item({first}, []), Item({second}, [])]));>)
in
(len-listing, len-enumerate)";
        let v = compile_v01_via_loader_with_metrics("itemize-lengths", src, &Mono)
            .expect("itemize.satyh (+ its transitive block/hdecoset/inline/list/option/path) should compile");
        let vs = match v {
            Value::Tuple(vs) => vs,
            other => panic!("expected a tuple, got {other:?}"),
        };
        assert_eq!(vs.len(), 2);
        let len_listing = as_length(vs[0].clone());
        let len_enumerate = as_length(vs[1].clone());
        assert!(
            len_listing.0 > 0.0,
            "expected +Itemize.listing's rendered block to have positive extent, got {len_listing:?}"
        );
        assert!(
            len_enumerate.0 > 0.0,
            "expected +Itemize.enumerate's rendered block to have positive extent, got {len_enumerate:?}"
        );
    });
}

// ============================================================================
// `proof.satyh` — sealed; `\derive`/`\derive-multi` are `math […]` command
// bindings with a LEADING `?(name:math-text, b:bool)` optional bundle
// (optional-arg-rows increment 3b-α, `typecheck.rs`'s `math_command_
// scheme_v01` — landed, see that increment's own seal-gate tests in
// `v01_opt_cmd_rows.rs`). Math-mode command APPLICATION has no `?(…)`
// bundle form (increment 3b-α's own test comment: "the call is
// necessarily unbundled"), so `name`/`b` simply default to `None` here —
// this probe only needs the two MANDATORY arguments to exercise the
// command end-to-end. The `list math-text` mandatory argument is built as
// an ordinary value OUTSIDE math mode (`[${...}, ${...}]`) and threaded in
// via `MathArg::ParenEscape` (`!(...)`, `cst_v1.rs`'s own `MathArg` enum);
// the trailing `math-text` argument uses the ordinary `{...}` math-group
// arg form.
// ============================================================================

#[test]
fn proof_bare_derive_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("proof-bare", "proof", "command \\derive");
    });
}

#[test]
fn proof_derive_computes_a_positive_natural_advance() {
    run_with_big_stack(|| {
        let src = "@require: v01-mini
@require: proof
@require: inline

let open V01Mini in
let ctx = get-initial-context 400pt (command \\math) in
let mlst = [${x}, ${y}] in
Inline.get-natural-advance (embed-math ctx (read-math ctx ${\\Proof.derive!(mlst){z}}))";
        let v = compile_v01_via_loader_with_metrics("proof-derive", src, &Mono)
            .expect("proof.satyh (+ its transitive graphics/inline/length/list/path) should compile");
        let len = as_length(v);
        assert!(
            len.0 > 0.0,
            "expected \\Proof.derive's rendered inline-boxes to have positive advance, got {len:?}"
        );
    });
}

// ============================================================================
// `md-ja.satyh` — sealed; a real document CLASS (`document : (| title,
// author |) -> block-text -> document`, its own `page-break`), so unlike
// `itemize`/`proof` above this package is exercised with a real, full,
// hand-written `.saty` document — mirroring `v01_stdlib.rs`'s font/hyph-
// unidata capstone tests AND `crates/rustyfi-cli/tests/e2e.rs`'s
// `v01_stdja_capstone_renders_to_extractable_text` (same shape, scoped to
// this crate's own `compile_document_v1` entry point rather than a full
// PDF render). Exercises: `+h1` (auto-numbering), `+p`, `\emph`/`\strong`,
// `\link`/`\reference` (the PDF-annotation frame + back-matter reference
// list), `\code`/inline monospace framing, `\hard-break`, `+ul-block`
// (which internally calls `+Itemize.listing?(break = true)(...)` — the
// batch's itemize -> md-ja dependency, exercised for real here), `+code`
// (this package's own ADAPTED implementation — see `md-ja.satyh`'s header
// banner — routing through `\Code.Default.code` since this port's `code`
// stand-in has no block-level `+code`/`Console`), `+quote`, `+hr`. `\img`
// is deliberately NOT exercised (would need a real loadable image file on
// disk, out of scope for this probe).
// ============================================================================

#[test]
fn mdja_bare_emph_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("mdja-bare", "md-ja", "command \\emph");
    });
}

#[test]
fn mdja_document_renders_via_real_loader_and_compile_document_v1() {
    run_with_big_stack(|| {
        let src = "@require: md-ja

MDJa.document (|
  title  = {MD-JA capstone},
  author = {The Vendoring Agents},
|) '<
  +MDJa.h1{Intro}<
    +MDJa.p{Hello \\MDJa.emph{world} and \\MDJa.strong{friends}. See \\MDJa.link(`http://example.com`){here}.}
  >
  +MDJa.p{
    Some code: \\MDJa.code(`x`); and a hard break follows.\\MDJa.hard-break;
    More body text.
    \\MDJa.reference(`tag1`)(`Example`)(Some((`Example Title`, `http://example.com`)));
  }
  +MDJa.ul-block(['<+MDJa.p{item one}>, '<+MDJa.p{item two}>]);
  +MDJa.code(`text`)(`let x = 1 in x`);
  +MDJa.quote<
    +MDJa.p{quoted text}
  >
  +MDJa.hr;
>";
        let doc = TempDoc::new("mdja-capstone", src);
        let opts = LoadOptions {
            lib_root: Some(lib_root()),
            version: RustyfiVersion::V0_1,
            ..Default::default()
        };
        let program = rustyfi_loader::load(&doc.0, &opts)
            .expect("md-ja.satyh + its full transitive @require: graph should load");
        assert!(
            program.files.len() > 1,
            "expected md-ja.satyh's transitive dependency graph plus the entry, got {} file(s)",
            program.files.len()
        );

        let doc_value = rustyfi_lang::compile_document_v1(&program.files, &Mono).expect(
            "md-ja.satyh should compile to a document: sealed module + `val mutable` counters + \
             optional-arg rows + the itemize/code cross-package dependency, all through real \
             elaborate/typecheck/sealing/eval",
        );
        assert!(!doc_value.pages.is_empty(), "expected at least one page");
        assert!(
            doc_value.pages.iter().any(|p| !p.lines.is_empty()),
            "expected at least one non-empty page"
        );
    });
}
