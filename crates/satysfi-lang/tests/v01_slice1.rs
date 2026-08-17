//! Slice 1 finale §8.2: the semantic-equivalence integration test —
//! `compile_document_v1` proven end-to-end (pre-flip: `satysfi-syntax`'s
//! `is_implemented()` gate is bypassed here by constructing `LoadedFile`/
//! `LoadedCst` directly, both of which have public fields), AND pinned
//! against a hand-written 0.0.6 twin (`v01-equiv-006.saty`, loaded through
//! the REAL loader + `merge_program`-equivalent + `compile_document_cst`)
//! for geometry equivalence — the plan's "same core IR, different surface,
//! proving no semantic drift" bar, expressed observably as equal page
//! geometry, equal per-page line counts, and equal placed-line x/baseline_y
//! sequences (span-insensitive, so it does not merely restate that the two
//! `Ast`s differ only in provenance).

use std::path::{Path, PathBuf};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::DocumentValue;
use satysfi_loader::{LoadOptions, LoadedCst, LoadedFile, LoadedProgram};
use satysfi_syntax::SatysfiVersion;

/// This repo's root, resolved relative to this crate's own manifest
/// directory (`crates/satysfi-lang/../..`).
fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)
}

/// A real (if crude) `FontMetrics` — the fixtures render actual ASCII text
/// (`The answer is 42.`, `Emphasis works.`), so `advance` must return
/// `Some` for ASCII, mirroring `stdlib_tier0.rs`'s/`eval.rs`'s own `Mono`
/// stub used the same way throughout this crate's test suite.
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

/// Merge a loader-resolved 0.0.6 program's preludes into one synthetic
/// `cst::File`, exactly like `satysfi-cli`'s private `merge_program`
/// (`main.rs`) — reproduced locally the same way `stdlib_tier0.rs` already
/// does (no `satysfi-cli` library target to import it from).
fn merge_program_006(program: LoadedProgram) -> satysfi_syntax::cst::File {
    fn as_v006(cst: LoadedCst) -> satysfi_syntax::cst::File {
        match cst {
            LoadedCst::V0_0_6(f) => f,
            LoadedCst::V0_1(_) => unreachable!("this test's 0.0.6 merge path only"),
        }
    }
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

#[test]
fn v01_document_compiles_and_matches_0_0_6_twin_geometry() {
    // ---- the V0_1 path, gate-bypassed (LoadedFile/LoadedCst are public) ----
    let lib_src = std::fs::read_to_string(repo("lib-satysfi/dist-v01/packages/v01-mini.satyh"))
        .expect("read v01-mini.satyh");
    let entry_src = std::fs::read_to_string(repo("crates/satysfi-cli/tests/fixtures/v01-minimal.saty"))
        .expect("read v01-minimal.saty");
    let files = vec![
        LoadedFile {
            path: repo("lib-satysfi/dist-v01/packages/v01-mini.satyh"),
            cst: LoadedCst::V0_1(
                satysfi_syntax::parse_file_v1(&lib_src).unwrap_or_else(|e| panic!("v01-mini.satyh: {e}")),
            ),
        },
        LoadedFile {
            path: repo("crates/satysfi-cli/tests/fixtures/v01-minimal.saty"),
            cst: LoadedCst::V0_1(
                satysfi_syntax::parse_file_v1(&entry_src).unwrap_or_else(|e| panic!("v01-minimal.saty: {e}")),
            ),
        },
    ];
    let mono = Mono;
    let (doc_v1, trials) = satysfi_lang::compile_document_v1_with_trials(&files, &mono)
        .unwrap_or_else(|e| panic!("compile_document_v1_with_trials: {e}"));
    assert_eq!(trials, 1, "no cross-references in this fixture — one trial suffices");
    assert_eq!(doc_v1.pages.len(), 1);
    assert!(
        doc_v1.pages[0].lines.len() >= 3,
        "two +p paragraphs + the footer line, got {}",
        doc_v1.pages[0].lines.len()
    );

    // ---- the 0.0.6 twin, through the REAL loader (@require: stdja-mini) ----
    let entry_006 = repo("crates/satysfi-cli/tests/fixtures/v01-equiv-006.saty");
    let lib_root_006 = repo("lib-satysfi");
    let program_006 = satysfi_loader::load(
        &entry_006,
        &LoadOptions {
            lib_root: Some(lib_root_006),
            version: SatysfiVersion::V0_0_6,
        },
    )
    .unwrap_or_else(|e| panic!("loading v01-equiv-006.saty: {e}"));
    let merged_006 = merge_program_006(program_006);
    let doc_006 = satysfi_lang::compile_document_cst(&merged_006, &mono)
        .unwrap_or_else(|e| panic!("compile_document_cst on the 0.0.6 twin: {e}"));

    assert_geometry_equivalent(&doc_v1, &doc_006);
}

/// The plan's "same core IR, different surface" bar, expressed observably:
/// equal page geometry, equal per-page line counts, and equal placed-line
/// x/baseline_y sequences — deliberately NOT full `PlacedLine`/`Ast`
/// equality (spans necessarily differ between the two source files, and
/// full-box-tree equality would also incidentally re-assert internal ids
/// that carry no semantic weight); this is strictly the geometry the plan's
/// proving test cares about, and strictly stronger than a syntactic
/// Ast-equality check would have been.
fn assert_geometry_equivalent(a: &DocumentValue, b: &DocumentValue) {
    assert_eq!(a.geometry, b.geometry, "page geometry must match exactly (both are A4)");
    assert_eq!(a.pages.len(), b.pages.len(), "page count must match");
    for (i, (pa, pb)) in a.pages.iter().zip(b.pages.iter()).enumerate() {
        assert_eq!(
            pa.lines.len(),
            pb.lines.len(),
            "page {i}: line count must match"
        );
        for (j, (la, lb)) in pa.lines.iter().zip(pb.lines.iter()).enumerate() {
            assert!(
                (la.x.0 - lb.x.0).abs() < 1e-6,
                "page {i} line {j}: x mismatch ({} vs {})",
                la.x.0,
                lb.x.0
            );
            assert!(
                (la.baseline_y.0 - lb.baseline_y.0).abs() < 1e-6,
                "page {i} line {j}: baseline_y mismatch ({} vs {})",
                la.baseline_y.0,
                lb.baseline_y.0
            );
        }
    }
}
