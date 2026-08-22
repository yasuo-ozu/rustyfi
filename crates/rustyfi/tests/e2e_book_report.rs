//! `std-ja-book` and `std-ja-report`, the two sibling
//! upstream SATySFi 0.1 document classes vendored at
//! `dist-v01/packages/std-ja-book.satyh` / `std-ja-report.satyh` (near-clones
//! of the `std-ja` capstone in `e2e.rs`). Disjoint from `e2e.rs`, mirroring
//! its `v01_stdja_capstone_renders_to_extractable_text` end to end: V0_1 lex
//! -> cst_v1 parse -> loader (`@require:`
//! transitive resolution) -> v1 lowering -> shared
//! elaborate/typecheck(V0_1)/sealing -> eval -> line break -> page break ->
//! PDF, `pdftotext`-asserted, skip-gated on a real DejaVu TrueType face
//! exactly like the std-ja capstone (both classes' footers/body need a real
//! font, not just base-14).
//!
//! `std-ja-report`'s `document` is also the only fixture in this port that
//! EXERCISES `page-break-multicolumn`'s V0_1 arm at run time; everywhere
//! else it is registered for type-table completeness only.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repo's `lib-rustyfi/` directory, resolved the same way `e2e.rs`'s own
/// `lib_root` does (relative to this crate's own manifest directory).
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

/// `annot`'s (and other deep `@require:` chains') CST recursion can overflow
/// the default 8 MiB test-thread stack — mirrors `e2e.rs`'s own helper of the
/// same name (duplicated: this crate has no shared test-support library
/// target).
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

/// Locate a real regular TrueType face (DejaVu) — both classes' footers use
/// an em-dash-flanked page number that isn't in base-14, and `std-ja-book`'s
/// `\figure`/title-page deco (this file never calls `\figure`, but the
/// footer alone already needs a real face), so a real font is required.
/// Duplicated from `e2e.rs`'s own `find_regular_ttf` (no shared test-support
/// library target in this crate).
fn find_regular_ttf() -> Option<PathBuf> {
    for family in ["DejaVuSerif", "DejaVuSans"] {
        if let Ok(output) = Command::new("fc-match")
            .args(["--format=%{file}", family])
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && Path::new(&path).is_file() && path.ends_with(".ttf") {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    for candidate in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/run/current-system/sw/share/fonts/truetype/DejaVuSans.ttf",
    ] {
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

/// `std-ja-book`: title-page deco, section rules,
/// `FootnoteScheme.main` (`\footnote`) all through the real V0_1 pipeline.
#[test]
fn v01_stdjabook_capstone_renders_to_extractable_text() {
    let font = match find_regular_ttf() {
        Some(p) => p,
        None => {
            eprintln!("skipping v01 std-ja-book capstone: no DejaVu TrueType font found");
            return;
        }
    };
    run_with_big_stack(move || {
        let entry =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v01-stdja-book.saty");
        let program = rustyfi_loader::load(
            &entry,
            &rustyfi_loader::LoadOptions {
                lib_root: Some(lib_root().join("dist-v01").join("packages")),
                version: rustyfi_syntax::RustyfiVersion::V0_1,
                ..Default::default()
            },
        )
        .expect(
            "std-ja-book.satyh + its full transitive @require: graph + v01-stdja-book.saty \
             must load",
        );

        let store =
            rustyfi_pdf::TtfFontStore::load(&font, None, None).expect("load DejaVu regular face");
        let doc = rustyfi_lang::compile_document_v1(&program.files, &store).expect(
            "the std-ja-book capstone must compile end-to-end: sealed module + records-in-\
             type-position + optional-arg-rows increments 1/2/3a + FootnoteScheme.main, \
             through real elaborate/typecheck/sealing/eval",
        );
        assert!(!doc.pages.is_empty(), "expected at least one page");
        assert!(
            doc.pages.iter().any(|p| !p.lines.is_empty()),
            "expected at least one non-empty page"
        );

        let bytes = rustyfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, &store, &doc.images)
            .expect("PDF rendering must succeed");
        assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
        assert!(
            bytes.windows(9).any(|w| w == b"FontFile2"),
            "expected an embedded TrueType font (FontFile2) in the capstone PDF"
        );

        let tmp = std::env::temp_dir().join(format!(
            "rustyfi-e2e-v01-stdja-book-{}.pdf",
            std::process::id()
        ));
        std::fs::write(&tmp, &bytes).unwrap();
        let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();
        match pdftotext {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                // The document record's title/author, rendered by
                // std-ja-book's own `+make-title` (real path/bezier
                // graphics deco).
                assert!(text.contains("SATySFi in Rust"), "missing title:\n{text}");
                assert!(
                    text.contains("The Vendoring Agents"),
                    "missing author:\n{text}"
                );
                // `+section`'s auto-numbering, unbundled (the optional-
                // argument None-defaulting path, live).
                assert!(
                    text.contains("1. Introduction"),
                    "missing section 1 title:\n{text}"
                );
                assert!(
                    text.contains("2. Conclusion"),
                    "missing section 2 title:\n{text}"
                );
                // Body text through `+StdJaBook.p`/`read-inline`.
                for word in ["quick", "brown", "fox"] {
                    assert!(
                        text.contains(word),
                        "pdftotext output missing {word:?} — the std-ja-book capstone must \
                         render extractable Latin body text:\n{text}"
                    );
                }
                // `\StdJaBook.footnote{…}` places `FootnoteScheme.main`'s
                // numbered superscript MARKER (`*1`) at the reference — this
                // proves the `\footnote` command + FootnoteScheme registration
                // run through the class.
                assert!(
                    text.contains("*1"),
                    "missing footnote superscript marker:\n{text}"
                );
                // The footnote BODY float, bottom-placed in the same column
                // by `chop_page` (`pagebreak.rs`): `FootnoteScheme.main`
                // wraps its `add-footnote` call in `Inline.no-break`, which
                // lowers to a `PureHorzBox::Frame` — `collect_footnotes_in_
                // box` must recurse into `Frame`'s `contents` (matching
                // upstream `pageInfo.ml`'s `ImHorzFrame` arm) for the marker
                // nested inside that frame to be found at all.
                assert!(
                    text.contains("A trailing footnote"),
                    "missing footnote body text:\n{text}"
                );
                assert!(text.contains('1'), "missing footer page number:\n{text}");
            }
            _ => eprintln!(
                "pdftotext unavailable; the PDF-header + FontFile2-embed checks already passed"
            ),
        }
        let _ = std::fs::remove_file(&tmp);
    });
}

/// `std-ja-report`: chapters, sections, theorem
/// environments (`+definition`/`+theorem`/`+proof`), and
/// `page-break-multicolumn`/`hook-page-break-block` (the V0_1 arm's FIRST
/// real exercise, see this file's module doc comment) all through the real
/// V0_1 pipeline.
#[test]
fn v01_stdjareport_capstone_renders_to_extractable_text() {
    let font = match find_regular_ttf() {
        Some(p) => p,
        None => {
            eprintln!("skipping v01 std-ja-report capstone: no DejaVu TrueType font found");
            return;
        }
    };
    run_with_big_stack(move || {
        let entry =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v01-stdja-report.saty");
        let program = rustyfi_loader::load(
            &entry,
            &rustyfi_loader::LoadOptions {
                lib_root: Some(lib_root().join("dist-v01").join("packages")),
                version: rustyfi_syntax::RustyfiVersion::V0_1,
                ..Default::default()
            },
        )
        .expect(
            "std-ja-report.satyh + its full transitive @require: graph + \
             v01-stdja-report.saty must load",
        );

        let store =
            rustyfi_pdf::TtfFontStore::load(&font, None, None).expect("load DejaVu regular face");
        let doc = rustyfi_lang::compile_document_v1(&program.files, &store).expect(
            "the std-ja-report capstone must compile end-to-end: sealed module + \
             page-break-multicolumn + hook-page-break-block + Ref.increment + \
             FootnoteScheme.main, through real elaborate/typecheck/sealing/eval",
        );
        assert!(!doc.pages.is_empty(), "expected at least one page");
        assert!(
            doc.pages.iter().any(|p| !p.lines.is_empty()),
            "expected at least one non-empty page"
        );

        let bytes = rustyfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, &store, &doc.images)
            .expect("PDF rendering must succeed");
        assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
        assert!(
            bytes.windows(9).any(|w| w == b"FontFile2"),
            "expected an embedded TrueType font (FontFile2) in the capstone PDF"
        );

        let tmp = std::env::temp_dir().join(format!(
            "rustyfi-e2e-v01-stdja-report-{}.pdf",
            std::process::id()
        ));
        std::fs::write(&tmp, &bytes).unwrap();
        let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();
        match pdftotext {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                // The document record's title/author.
                assert!(text.contains("SATySFi in Rust"), "missing title:\n{text}");
                assert!(
                    text.contains("The Vendoring Agents"),
                    "missing author:\n{text}"
                );
                // `+chapter`'s auto-numbering (`chapter-scheme`'s
                // `Ref.increment num-chapter`).
                assert!(
                    text.contains("1. Introduction"),
                    "missing chapter 1 title:\n{text}"
                );
                assert!(
                    text.contains("2. Conclusion"),
                    "missing chapter 2 title:\n{text}"
                );
                // `+section`'s auto-numbering nested inside chapter 1
                // (`Ref.increment num-section`).
                assert!(
                    text.contains("1.1. Background"),
                    "missing section 1.1 title:\n{text}"
                );
                // Body text through `+StdJaReport.p`/`read-inline`.
                for word in ["quick", "brown", "fox"] {
                    assert!(
                        text.contains(word),
                        "pdftotext output missing {word:?} — the std-ja-report capstone must \
                         render extractable Latin body text:\n{text}"
                    );
                }
                // The theorem environments' auto-generated category labels
                // (`theorem-scheme`'s `{#category; #it-num;}`).
                assert!(
                    text.contains("Definition"),
                    "missing Definition label:\n{text}"
                );
                assert!(text.contains("Theorem"), "missing Theorem label:\n{text}");
                assert!(text.contains("Proof"), "missing Proof label:\n{text}");
                // `\StdJaReport.footnote{…}` places FootnoteScheme's numbered
                // superscript MARKER (`*1`).
                assert!(
                    text.contains("*1"),
                    "missing footnote superscript marker:\n{text}"
                );
                // The footnote BODY float, bottom-placed in the same column
                // — see the book test above for the `PureHorzBox::Frame`
                // traversal it depends on.
                assert!(
                    text.contains("A trailing footnote"),
                    "missing footnote body text:\n{text}"
                );
                assert!(text.contains('1'), "missing footer page number:\n{text}");
            }
            _ => eprintln!(
                "pdftotext unavailable; the PDF-header + FontFile2-embed checks already passed"
            ),
        }
        let _ = std::fs::remove_file(&tmp);
    });
}
