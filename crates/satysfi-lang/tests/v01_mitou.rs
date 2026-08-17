//! Mitou conversion pass: `mitou-report.satyh`/`mitou-detail.satyh` — the
//! two upstream "mitou" (未踏) report document classes, `MitouReport`/
//! `MitouDetail`. UNLIKE every other document class vendored so far
//! (`std-ja`/`std-ja-book`/`std-ja-report`/`md-ja`), these two are NOT
//! 0.1-dialect siblings of an already-envelope-shaped upstream package —
//! upstream (`saphe-split@b836d512:lib-satysfi/packages/mitou-{report,
//! detail}.satyh`) is a genuine pre-envelope 0.0-syntax leftover whose
//! `@require: pervasives`/`gr`/`math`/`color` targets flat module names
//! that upstream's own Aug-2024 stdlib refactor deleted (see each
//! vendored file's own header banner for the full archaeology + the
//! complete 0.0->0.1 delta catalogue — dialect AND stdlib-API rebasing,
//! not just a transliteration). PROVEN here through the real production
//! loader (`satysfi_loader::load`, `lib_root = dist-v01/packages`,
//! `SatysfiVersion::V0_1`) — not merely parsed.
//!
//! Harness copied from `v01_itemize_proof_mdja.rs`/`v01_stdlib.rs`
//! (reproduced locally per those files' own established convention — no
//! shared test-support library target exists in this crate): real loader
//! -> per-file `lower_file_v1` prelude concatenation -> `elaborate_program`
//! -> `typecheck` -> `eval::Interp::eval`/`compile_document_v1`, plus the
//! same `assert_bare_access_unbound` qualified-export negative probe every
//! vendored package gets once (`v01_modules.rs`'s `TopBinding::Module`
//! wrapping makes every member reachable only as `Pkg.member`).
//!
//! Both classes' `document` bakes in UNCONDITIONAL Japanese text (the
//! title page's `年度未踏IT人材・育成事業`/`成果報告書`, `mitou-report`'s
//! always-rendered table-of-contents `目次`, `\figure`'s `図` caption
//! prefix) — unlike `std-ja-book`'s/`std-ja-report`'s own capstones, which
//! carefully stay Latin-only by never calling `\figure`, there is no way
//! to exercise either `document` without hitting CJK glyphs. So, exactly
//! like `v01_itemize_proof_mdja.rs`'s own `md-ja` capstone (same
//! situation, same fix), the `Mono` stub below advances EVERY character
//! (not just ASCII) rather than trying to source a real CJK-covering TTF
//! font — these are `satysfi-lang` "value bar" tests (real
//! `compile_document_v1` page/line assertions), not `satysfi-cli`'s
//! `pdftotext`-asserted PDF-embedding e2e tier, so no real font file is
//! needed at all.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use satysfi_loader::{LoadOptions, LoadedCst, LoadedFile};
use satysfi_syntax::SatysfiVersion;

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-satysfi/dist-v01/packages")
}

struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-lang-v01-mitou-{tag}-{}-{}.saty",
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

/// A real, FULLY-COVERING `FontMetrics` — both capstones below render
/// unavoidable CJK text (see this file's module doc comment), so unlike
/// `v01_stdlib.rs`'s ASCII-only `Mono`, this stub advances EVERY
/// character. Mirrors `v01_itemize_proof_mdja.rs`'s own `Mono` (same
/// rationale, same fix, for the same reason: `md-ja`'s back-matter heading
/// forced the same widening there).
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

fn as_v01(f: &LoadedFile) -> &satysfi_syntax::cst_v1::FileV1 {
    match &f.cst {
        LoadedCst::V0_1(cst) => cst,
        LoadedCst::V0_0_6(_) => unreachable!("this test's helper is V0_1-only"),
    }
}

/// Load `src` through the REAL multi-file loader, assemble the synthetic
/// `cst::File` exactly the way `compile_document_v1_with_trials` does,
/// then elaborate -> typecheck -> eval directly to a `Value` (mirrors
/// `v01_stdlib.rs`'s/`v01_itemize_proof_mdja.rs`'s own
/// `compile_v01_via_loader`, with ONE fix: those two files build their
/// `Scope` with plain `Scope::new(env.names())`, which defaults to
/// `SatysfiVersion::V0_0_6` (`elaborate.rs`'s own `Scope::new` doc
/// comment) — harmless for THEIR packages (none of `itemize`/`proof`/
/// `md-ja`'s OWN bare-unbound probes transitively `@require:` a package
/// whose `?(l = x)`-shaped command definitions get elaborated along the
/// way), but `mitou-report`/`mitou-detail` both transitively `@require:
/// math`, and `math.satyh`'s own `+math ?(tag = tagopt)`/`\eqn ?(tag =
/// tagopt)` command definitions DO hit `elaborate.rs`'s `fun_rows_to_ast`
/// version gate, which then rejects them as "compiled as 0.0.6". Using
/// `Scope::new_with_version(names, V0_1)` here (exactly what the real
/// `compile_document_v1` path already does, and what `Scope::new_with_
/// version`'s own doc comment says the V0_1 compile path is FOR) fixes
/// it — this is a latent gap in the copied-template helper, not a defect
/// in `math.satyh` or in `mitou-report.satyh`/`mitou-detail.satyh`.
fn compile_v01_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    let doc = TempDoc::new(tag, src);
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        version: SatysfiVersion::V0_1,
        ..Default::default()
    };
    let program = satysfi_loader::load(&doc.0, &opts).map_err(|e| format!("load: {e}"))?;

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
        satysfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => return Err("entry must parse as a V0_1 document".to_string()),
    };
    let file = satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(satysfi_syntax::leaf::KwIn(satysfi_syntax::Span::default())),
        body: Some(body),
        eoi,
    };

    let env = primitives::base_env_with_version(SatysfiVersion::V0_1);
    let scope = elaborate::Scope::new_with_version(env.names(), SatysfiVersion::V0_1);
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, SatysfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(&Mono);
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

/// `annot`'s (and other deep `@require:` chains') CST recursion can
/// overflow the default 8 MiB test-thread stack — mirrors
/// `v01_stdlib.rs`'s/`v01_itemize_proof_mdja.rs`'s own helper of the same
/// name.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

#[test]
fn mitou_report_bare_ref_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("mitou-report-bare", "mitou-report", "command \\ref");
    });
}

#[test]
fn mitou_detail_bare_ref_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("mitou-detail-bare", "mitou-detail", "command \\ref");
    });
}

/// `MitouReport`'s full command surface through the real loader +
/// `compile_document_v1`: `document`'s mandatory 5-field record (`project`/
/// `year`/`creators`/`manager`/`jouzai-number`), `+section`/`+subsection`
/// (both the explicit-`?(label = …)` and the auto-generated-label paths),
/// `+p`/`+pn`, `\ref`/`\ref-page` (cross-reference round-trip through the
/// page-break "trials" fixed point), `\figure` (the `Block.form-
/// paragraph`/`Inline.get-natural-advance`/`List.fold`/`List.fold-adjacent`
/// conversion deltas all fire here — the title page's creator table uses
/// `List.fold-adjacent` + `List.reverse` + `tabular`, the TOC uses
/// `Inline.get-natural-advance`'s dot-leaders), and `\emph`.
#[test]
fn mitou_report_document_renders_via_real_loader_and_compile_document_v1() {
    run_with_big_stack(|| {
        let src = "@require: mitou-report

MitouReport.document (|
  project       = {Rust Widget},
  year          = 2024,
  creators      = [{Ada}, {Grace}],
  manager       = {Linus},
  jouzai-number = 42,
|) '<
  +MitouReport.section ?(label = `intro`) {Introduction}<
    +MitouReport.p{
      This is a converted upstream SATySFi 0.1 document class, mitou-report, rendered end to end through the Rust port. Cross references: \\MitouReport.ref(`intro`); on page \\MitouReport.ref-page(`intro`); with \\MitouReport.emph{emphasis}.\\MitouReport.figure ?(label = `fig1`){A figure caption.}<+MitouReport.p{Figure body text.}>
    }
    +MitouReport.pn{
      A second, non-indented paragraph.
    }
    +MitouReport.subsection{A Subsection}<
      +MitouReport.p{
        Body text inside a subsection, with an auto-generated label.
      }
    >
  >
>";
        let doc = TempDoc::new("report-capstone", src);
        let opts = LoadOptions {
            lib_root: Some(lib_root()),
            version: SatysfiVersion::V0_1,
            ..Default::default()
        };
        let program = satysfi_loader::load(&doc.0, &opts)
            .expect("mitou-report.satyh + its full transitive @require: graph should load");
        assert!(
            program.files.len() > 1,
            "expected mitou-report.satyh's transitive dependency graph (list/inline/block/math) \
             plus the entry, got {} file(s)",
            program.files.len()
        );

        let doc_value = satysfi_lang::compile_document_v1(&program.files, &Mono).expect(
            "mitou-report.satyh should compile to a document: sealed module + `val mutable` \
             counters + optional-arg rows + List.fold/fold-adjacent/reverse + Inline.get-natural-\
             advance + Block.form-paragraph + the tabular title-page table, all through real \
             elaborate/typecheck/sealing/eval",
        );
        assert!(!doc_value.pages.is_empty(), "expected at least one page");
        assert!(
            doc_value.pages.len() >= 2,
            "expected at least a title page + a TOC/body page (document's own +++ clear-page \
             between title/toc/main), got {}",
            doc_value.pages.len()
        );
        assert!(
            doc_value.pages.iter().any(|p| !p.lines.is_empty()),
            "expected at least one non-empty page"
        );
    });
}

/// `MitouDetail`'s full command surface: `document`'s 2-field record
/// (`project`/`subtitle`), `+section`/`+section-no-number`/`+subsection`,
/// `+p`/`+pn`, `\ref`/`\ref-page`, `\figure`. (`MitouDetail` has no `\emph`
/// in its sig — not called here.)
#[test]
fn mitou_detail_document_renders_via_real_loader_and_compile_document_v1() {
    run_with_big_stack(|| {
        let src = "@require: mitou-detail

MitouDetail.document (|
  project  = {Rust Widget: Detailed Report},
  subtitle = {A Deeper Dive},
|) '<
  +MitouDetail.section ?(label = `intro`) {Introduction}<
    +MitouDetail.p{
      This is a converted upstream SATySFi 0.1 document class, mitou-detail, rendered end to \
      end through the Rust port. Cross references: \\MitouDetail.ref(`intro`); on page \\MitouDetail.ref-page(`intro`);\\MitouDetail.figure ?(label = `fig1`){A figure caption.}<+MitouDetail.p{Figure body text.}>
    }
    +MitouDetail.pn{
      A second, non-indented paragraph.
    }
    +MitouDetail.subsection{A Subsection}<
      +MitouDetail.p{
        Body text inside a subsection, with an auto-generated label.
      }
    >
  >
  +MitouDetail.section-no-number{Unnumbered Section}<
    +MitouDetail.p{ Body text under an unnumbered section. }
  >
>";
        let doc = TempDoc::new("detail-capstone", src);
        let opts = LoadOptions {
            lib_root: Some(lib_root()),
            version: SatysfiVersion::V0_1,
            ..Default::default()
        };
        let program = satysfi_loader::load(&doc.0, &opts)
            .expect("mitou-detail.satyh + its full transitive @require: graph should load");
        assert!(
            program.files.len() > 1,
            "expected mitou-detail.satyh's transitive dependency graph (list/inline/block/math) \
             plus the entry, got {} file(s)",
            program.files.len()
        );

        let doc_value = satysfi_lang::compile_document_v1(&program.files, &Mono).expect(
            "mitou-detail.satyh should compile to a document: sealed module + `val mutable` \
             counters + optional-arg rows + List.fold + Block.form-paragraph, all through real \
             elaborate/typecheck/sealing/eval",
        );
        assert!(!doc_value.pages.is_empty(), "expected at least one page");
        assert!(
            doc_value.pages.iter().any(|p| !p.lines.is_empty()),
            "expected at least one non-empty page"
        );
    });
}
