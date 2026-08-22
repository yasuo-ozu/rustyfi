//! Vendoring wave-4 Batch B: `itemize.satyh`/`proof.satyh`/`md-ja.satyh` —
//! three more upstream SATySFi 0.1 packages, transliterated from
//! `saphe-split@b836d512` (see each package's own header banner for its
//! exact upstream path + deltas), PROVEN through the real production
//! loader. Dependency order: `itemize` (no deps) -> `proof` (independent)
//! -> `md-ja` (LAST: `@require:`s `itemize`, transitively `math`/`code`/
//! `annot`/`hyph-english`/`unidata`/the 4 font stand-ins).
//!
//! All three packages export ONLY command bindings — no plain top-level
//! value — so the "value" probes below build a real `context`, invoke the
//! command through `read-block`/`read-inline`/`read-math`, and extract a
//! computed `Length` via `get-natural-length`/`Inline.get-natural-advance`
//! (`itemize`/`proof`), or a rendered `DocumentValue` via
//! `compile_document_v1` (`md-ja`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedFile};
use rustyfi_syntax::RustyfiVersion;

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi/dist-v01/packages")
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

/// Stub — never consulted; the itemize/proof value probes only measure
/// natural lengths of ASCII-free or near-empty content.
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

/// A FULLY-COVERING `FontMetrics` (advances EVERY character, unlike the
/// ASCII-only `Mono` elsewhere): `md-ja`'s document renders a CJK back-
/// matter heading (`参考文献`) whenever `\reference` has populated
/// `reference-acc`, which this capstone exercises — an ASCII-only stub
/// would fail glyph lookup on `参`.
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
        LoadedCst::V0_0(_) => unreachable!("this test's helper is V0_1-only"),
    }
}

/// Reproduces `compile_document_v1_with_trials` (`lib.rs:165-195`).
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
    // `Scope::new` defaults to 0.0.6, which rejects the `?(l = x)`
    // labeled-optional path these packages all use — hence the explicit
    // V0_1 scope.
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new_with_version(&store, env.names(), RustyfiVersion::V0_1);
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, RustyfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(metrics);
    // `coerce_graphics_result` forks on `interp.version` at EVAL time (not
    // primitive selection, already resolved by `base_env_with_version`):
    // graphics callbacks (fired by itemize's bullets, proof's inference
    // bar) return `graphics` under V0_1 but `list graphics` under the
    // default V0_0 — must be set or these eval-error "expected list, got
    // graphics".
    interp.version = RustyfiVersion::V0_1;
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&elaborated.body, &store))
        .map_err(|e| format!("eval: {e}"))
}

/// The qualified-export negative probe every vendored package gets once:
/// `bare_expr` without its `Pkg.` qualifier must fail "unbound variable".
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

/// Needs a big stack: every package here transitively `@require:`s
/// `list.satyg` (280+ lines), exceeding syan's default recursion depth.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

// `itemize.satyh` — sealed; `+listing`/`\listing`/`+enumerate`/
// `\enumerate` are ALL commands (no plain value export), so the probe
// below builds a `context` and measures rendered `block-boxes` via the
// bare `get-natural-length` primitive.

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

// `proof.satyh` — sealed; `\derive`/`\derive-multi` are `math […]`
// commands with a LEADING `?(name:math-text, b:bool)` optional bundle
// (`typecheck.rs`'s `math_command_scheme_v01`; seal-gate tests live in
// `v01_opt_cmd_rows.rs`). Math-mode command APPLICATION has no `?(…)`
// bundle form, so `name`/`b` default to `None` here — this probe only
// exercises the two mandatory arguments. The `list math-text` argument is
// built outside math mode and threaded in via `MathArg::ParenEscape`
// (`!(...)`).

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
        let v = compile_v01_via_loader_with_metrics("proof-derive", src, &Mono).expect(
            "proof.satyh (+ its transitive graphics/inline/length/list/path) should compile",
        );
        let len = as_length(v);
        assert!(
            len.0 > 0.0,
            "expected \\Proof.derive's rendered inline-boxes to have positive advance, got {len:?}"
        );
    });
}

// `md-ja.satyh` — sealed; a real document CLASS (`document : (| title,
// author |) -> block-text -> document`), so unlike `itemize`/`proof` this
// is exercised with a real, full document rather than a bare probe.
// `+code` is this package's own ADAPTED implementation, routing through
// `\Code.Default.code` since this port's `code` stand-in has no block-
// level `+code`/`Console`. `\img` is deliberately NOT exercised (needs a
// real loadable image file on disk).

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
