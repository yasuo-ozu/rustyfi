//! End-to-end: compile each fixture `.saty` to a PDF through the real
//! multi-file loader (`satysfi_loader::load` + the same prelude-merge the
//! CLI's `merge_program` does), then verify the text — via pdftotext when
//! available, otherwise by grepping the uncompressed content streams for the
//! `Tj` string operands.
//!
//! Phase 4: `document`/`+p`/`\emph` are no longer hardcoded Rust natives —
//! every fixture now `@require:`s the real `stdja-mini` stdlib package
//! (`lib-satysfi/dist/packages/stdja-mini.satyh`), so every compile below
//! goes through the loader with a `lib_root` pointing at this repo's
//! `lib-satysfi/`, not `satysfi_lang::compile_document` directly.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repo's `lib-satysfi/` directory, resolved the same way the task
/// describes for tests: relative to this crate's own manifest directory.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-satysfi")
}

/// Load `entry` and its full `@require:`/`@import:` dependency graph
/// (against [`lib_root`]), then concatenate the dependency-ordered library
/// preludes ahead of the entry document's own prelude — exactly
/// `satysfi-cli`'s `merge_program` (src/main.rs).
fn load_and_merge(entry: &Path) -> satysfi_syntax::cst::File {
    let program = satysfi_loader::load(
        entry,
        &satysfi_loader::LoadOptions {
            lib_root: Some(lib_root()),
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("failed to load {}: {e}", entry.display()));

    let mut files = program.files;
    let entry_file = files.pop().expect("loader always yields the entry last");
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry_file.cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_file.cst.in_kw,
        body: entry_file.cst.body,
        eoi: entry_file.cst.eoi,
    }
}

fn compile_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics).expect("fixture must compile");
    assert!(!doc.pages.is_empty());
    assert!(
        doc.pages[0].lines.len() >= 3,
        "the long paragraph must wrap: got {} lines",
        doc.pages[0].lines.len()
    );
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).expect("PDF rendering must succeed")
}

#[test]
fn fixture_compiles_to_valid_pdf_with_expected_text() {
    let bytes = compile_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let tmp = std::env::temp_dir().join(format!("satysfi-rust-e2e-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();

    let pdftotext = Command::new("pdftotext")
        .arg(&tmp)
        .arg("-")
        .output();

    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            for expected in [
                "Hello, world!",
                "SATySFi-in-Rust",
                "second paragraph",
                "end to end",
            ] {
                assert!(
                    text.contains(expected),
                    "pdftotext output missing {expected:?}:\n{text}"
                );
            }
        }
        _ => {
            // Fallback: content streams are uncompressed, so the Tj string
            // operands are directly visible in the bytes.
            let hay = String::from_utf8_lossy(&bytes);
            // `\emph{SATySFi-in-Rust}.` sets the emphasized word (oblique) and
            // the trailing `.` as separate text runs, so the period is not part
            // of this operand.
            for expected in ["(Hello,)", "(world!)", "(SATySFi-in-Rust)"] {
                assert!(
                    hay.contains(expected),
                    "content stream missing {expected:?}"
                );
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

fn compile_phase2_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase2.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc =
        satysfi_lang::compile_document_cst(&merged, &metrics).expect("phase2 fixture must compile");
    assert_eq!(doc.pages.len(), 1);
    assert!(
        doc.pages[0].lines.len() >= 3,
        "expected at least one line per +p paragraph, got {}",
        doc.pages[0].lines.len()
    );
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).expect("PDF rendering must succeed")
}

/// End-to-end coverage for the phase-2 elaborator (operator-precedence fold,
/// `let-rec`, `match`, and both `let-inline` forms) via a real `.saty`
/// document, checked the same way as the milestone-1 fixture: pdftotext
/// when available, otherwise a direct scan of the uncompressed content
/// stream's `Tj` string operands.
#[test]
fn phase2_fixture_compiles_and_renders_expected_text() {
    let bytes = compile_phase2_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let tmp =
        std::env::temp_dir().join(format!("satysfi-rust-e2e-phase2-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();

    let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();

    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            for expected in [
                "Bracketed text via let-inline.",
                "Announced text via the lightweight let-inline form.",
                "Countdown complete.",
            ] {
                assert!(
                    text.contains(expected),
                    "pdftotext output missing {expected:?}:\n{text}"
                );
            }
            assert!(
                !text.contains("Countdown incomplete."),
                "the let-rec/match should have selected the 'finished' branch"
            );
        }
        _ => {
            let hay = String::from_utf8_lossy(&bytes);
            for expected in ["(Bracketed)", "(Announced)", "(Countdown)", "(complete.)"] {
                assert!(hay.contains(expected), "content stream missing {expected:?}");
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

fn compile_phase2b_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase2b.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("phase2b fixture must compile");
    assert_eq!(doc.pages.len(), 1);
    assert!(
        !doc.pages[0].lines.is_empty(),
        "expected at least one line, got {}",
        doc.pages[0].lines.len()
    );
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).expect("PDF rendering must succeed")
}

/// End-to-end coverage for the phase-2b elaborator additions (a module +
/// qualified reference, `#label` field access, `let-mutable`/`while`/
/// `before`-built countdown string, and `+p`) via a real `.saty` document,
/// checked the same way as the earlier fixtures: pdftotext when available,
/// otherwise a direct scan of the uncompressed content stream's `Tj` string
/// operands.
#[test]
fn phase2b_fixture_compiles_and_renders_expected_text() {
    let bytes = compile_phase2b_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let tmp =
        std::env::temp_dir().join(format!("satysfi-rust-e2e-phase2b-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();

    let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();

    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            for expected in ["Countdowncomplete."] {
                assert!(
                    text.replace(char::is_whitespace, "").contains(expected),
                    "pdftotext output missing {expected:?}:\n{text}"
                );
            }
            assert!(
                !text.contains("incomplete"),
                "the let-mutable/while countdown should have reached zero: {text}"
            );
        }
        _ => {
            let hay = String::from_utf8_lossy(&bytes);
            for expected in ["(Countdown)", "(complete.)"] {
                assert!(hay.contains(expected), "content stream missing {expected:?}");
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

/// A non-fixture source string, compiled through the same loader path by
/// writing it to a temp file that itself `@require:`s `stdja-mini` — `\emph`
/// is no longer a Rust native, so exercising it (even for this error-path
/// test) needs the real package.
#[test]
fn non_winansi_text_errors_politely() {
    let tmp = std::env::temp_dir().join(format!(
        "satysfi-rust-e2e-nonwinansi-{}.saty",
        std::process::id()
    ));
    std::fs::write(
        &tmp,
        "@require: stdja-mini\ndocument (||) '< +p { こんにちは } >",
    )
    .unwrap();

    let merged = load_and_merge(&tmp);
    let metrics = satysfi_pdf::Base14Metrics;
    let err = satysfi_lang::compile_document_cst(&merged, &metrics).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("WinAnsi") || msg.contains("not available"),
        "unhelpful error: {msg}"
    );
    let _ = std::fs::remove_file(&tmp);
}

fn compile_graphics_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graphics.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("graphics fixture must compile");
    assert_eq!(doc.pages.len(), 1);
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).expect("PDF rendering must succeed")
}

/// End-to-end coverage for the Slice 1 graphics primitives
/// (`docs/plans/graphics-subsystem.md`): `start-path`/`line-to`/
/// `close-with-line` build a 20pt-square `path`, `fill`/`stroke` turn it
/// into `graphics`, and a local `\graphics` command (`inline-graphics`)
/// places it on the page. Checked by scanning the uncompressed content
/// stream for the path operators the rectangle must produce — the box's
/// local path coordinates are exact regardless of where real line/page
/// layout ends up placing the box (`place_graphics` translates the whole
/// box via one `cm`, never per-coordinate).
#[test]
fn graphics_fixture_compiles_and_renders_path_operators() {
    let bytes = compile_graphics_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let hay = String::from_utf8_lossy(&bytes);
    // Path construction: move to the rectangle's start, three line-tos, then
    // `close_path` (`h`, zero operands, so bounded by newlines not spaces).
    for op in ["0 0 m", "20 0 l", "20 20 l", "0 20 l", "\nh\n"] {
        assert!(hay.contains(op), "content stream missing {op:?}:\n{hay}");
    }
    // Fill (even-odd — upstream's `op_f'`) in RGB red, then a 1pt gray
    // stroke — each re-emits its own copy of the path before painting it.
    for op in ["1 0 0 rg", "f*", "1 w", "0 G", "\nS\n"] {
        assert!(hay.contains(op), "content stream missing {op:?}:\n{hay}");
    }
    // The whole box is placed via a single `cm` translate, not a per-
    // coordinate flip.
    assert!(
        hay.contains(" cm\n"),
        "content stream missing the box's placement transform:\n{hay}"
    );
}

/// `list.satyg` + `stdja-mini` + this fixture's own `\tabular` definition
/// (two variant-ctor-bearing closures, `NormalCell`/`MultiCell`) push the
/// merged program's parse tree past the default thread stack's depth budget
/// through syan's recursive-descent parser — the same reason
/// `satysfi-lang/tests/stdlib_tier0.rs`'s `run_with_big_stack` and
/// `satysfi-syntax/tests/roundtrip.rs`'s deep-nesting test spawn a
/// bigger-stack thread for comparably-sized inputs. `Vec<u8>` (the rendered
/// PDF bytes) is `Send`, so the whole compile-and-render can run on that
/// thread and its result join back out normally (unlike `stdlib_tier0.rs`'s
/// `Value`, which holds non-`Send` `Rc`s and must never cross the join).
fn compile_table_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/table.saty");
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let merged = load_and_merge(&fixture);
            let metrics = satysfi_pdf::Base14Metrics;
            let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
                .expect("table fixture must compile");
            assert_eq!(doc.pages.len(), 1);
            satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images)
                .expect("PDF rendering must succeed")
        })
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)")
}

/// End-to-end coverage for the Slice 1 table subsystem
/// (`docs/plans/table-subsystem.md`): a self-contained, positional
/// `\tabular` (the new `cell` type + `tabular` primitive) renders a 2x2
/// ruled grid — both the four cells' text and the grid rules must land in
/// the same content stream, through the composite-box `emit_box` writer arm
/// (§4 of the plan, the subsystem's biggest risk).
#[test]
fn table_fixture_compiles_and_renders_cell_text_and_rules() {
    let bytes = compile_table_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let hay = String::from_utf8_lossy(&bytes);
    // Cell text: each of the 2x2 grid's four letters is a `Tj` run.
    for letter in ["(A) Tj", "(B) Tj", "(C) Tj", "(D) Tj"] {
        assert!(hay.contains(letter), "content stream missing cell text {letter:?}:\n{hay}");
    }
    // Rule path ops (`m`/`l`), a stroke (`S`), a set line width (`w`), and a
    // gray color op (`g`/`G`) — the ruled grid drawn through the existing
    // `place_graphics`.
    for op in [" m\n", " l\n", "\nS\n", " w\n"] {
        assert!(hay.contains(op), "content stream missing rule op {op:?}:\n{hay}");
    }
    assert!(
        hay.contains("0 G") || hay.contains(" g\n"),
        "content stream missing a gray color op:\n{hay}"
    );
}

/// Multi-file loading through the loader crate: a document `@require:`s the
/// `stdja-mini` stdlib package and `@import:`s a local library, whose
/// bindings (a value, a command, a function) all resolve.
#[test]
fn multifile_import_compiles_and_renders() {
    let entry = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multifile/main.saty");
    let program = satysfi_loader::load(
        &entry,
        &satysfi_loader::LoadOptions {
            lib_root: Some(lib_root()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        program.files.len(),
        3,
        "stdja-mini.satyh + helpers.satyh + main.saty"
    );

    let merged = load_and_merge(&entry);

    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics).unwrap();
    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).unwrap();

    let tmp = std::env::temp_dir().join(format!("satysfi-rust-e2e-mf-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();
    if let Ok(out) = Command::new("pdftotext").arg(&tmp).arg("-").output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains("Imported command works."), "missing: {text}");
            assert!(text.contains("Twice twenty-one is 42 indeed."), "missing: {text}");
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

fn compile_math_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/math.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("math fixture must compile (docs/plans/math-engine.md §Slice 1)");
    assert_eq!(doc.pages.len(), 1);
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).expect("PDF rendering must succeed")
}

/// The nearest preceding `Tf`-size / `Td`-y pair before `glyph_tj` (e.g.
/// `"(2) Tj"`) in an uncompressed content stream — each math glyph is its
/// own `BT / Tf / Td / Tj / ET` run (`place_math`), so the immediately
/// preceding `Tf`/`Td` lines are exactly that glyph's own font size and
/// placed y.
fn glyph_size_and_y(hay: &str, glyph_tj: &str) -> (f32, f32) {
    let tj_pos = hay
        .find(glyph_tj)
        .unwrap_or_else(|| panic!("content stream missing {glyph_tj:?}:\n{hay}"));
    let before = &hay[..tj_pos];
    let td_line = before
        .rsplit('\n')
        .find(|l| l.ends_with(" Td"))
        .expect("a Td line must precede every Tj");
    let tf_line = before
        .rsplit('\n')
        .find(|l| l.ends_with(" Tf"))
        .expect("a Tf line must precede every Tj");
    let y: f32 = td_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("Td line must be '<x> <y> Td'");
    let size: f32 = tf_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("Tf line must be '/F<n> <size> Tf'");
    (size, y)
}

/// End-to-end coverage for the Slice 1 math engine
/// (`docs/plans/math-engine.md`): `${x^2}` and `${a+b}` are core `${…}`
/// syntax (no `@require: math` — the math package isn't loaded yet, §G).
/// Checked directly against the uncompressed content stream (not
/// `pdftotext`, which discards size/position) since the acceptance is the
/// *offset and scale* of the superscript, not just which characters appear.
#[test]
fn math_fixture_renders_superscript_raised_and_scaled() {
    let bytes = compile_math_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
    let hay = String::from_utf8_lossy(&bytes);

    // `read_math` emits one glyph per char, each its own `Tj` operand.
    for glyph in ["(x)", "(2)", "(a)", "(+)", "(b)"] {
        assert!(
            hay.contains(glyph),
            "content stream missing {glyph:?}:\n{hay}"
        );
    }

    let (base_size, base_y) = glyph_size_and_y(&hay, "(x) Tj");
    let (sup_size, sup_y) = glyph_size_and_y(&hay, "(2) Tj");

    assert!(
        (base_size - 12.0).abs() < 0.1,
        "base 'x' should be set at the context's 12pt: got {base_size}"
    );
    assert!(
        sup_size < base_size,
        "superscript '2' must be set smaller than the base: {sup_size} vs {base_size}"
    );
    assert!(
        sup_y > base_y,
        "superscript '2' must be raised above the base's baseline \
         (PDF y is up): {sup_y} vs {base_y}"
    );
}

fn compile_hook_page_fixture() -> (Vec<u8>, u32) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hook-page.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let (doc, trials) = satysfi_lang::compile_document_cst_with_trials(&merged, &metrics)
        .expect("hook-page fixture must compile");
    assert_eq!(doc.pages.len(), 1);
    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images)
        .expect("PDF rendering must succeed");
    (bytes, trials)
}

/// End-to-end coverage for the Slice 1 page-break-hook + cross-reference
/// callback foundation (docs/plans/hooks-annotations-crossref.md): a
/// `hook-page-break` closure registers the page's own (final) page number
/// as a cross-reference; a `get-cross-reference` read-back on a later trial
/// renders it as text. Trial 1 registers a fresh key (`changed`), forcing
/// trial 2, where the read-back resolves and `embed-string` renders it.
/// Asserting `trials == 2` is the load-bearing check the plan calls for: a
/// one-pass fluke would also emit "1" if the read happened to pre-resolve,
/// but would prove nothing about the callback seam actually firing.
#[test]
fn hook_page_fixture_fires_the_hook_and_renders_the_final_page_number() {
    let (bytes, trials) = compile_hook_page_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
    assert_eq!(
        trials, 2,
        "the cross-ref fixpoint must take exactly 2 trials to converge"
    );

    let tmp = std::env::temp_dir()
        .join(format!("satysfi-rust-e2e-hookpage-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();

    let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();
    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.replace(char::is_whitespace, "").contains("Pagenumber:1"),
                "pdftotext output missing the rendered page number:\n{text}"
            );
        }
        _ => {
            let hay = String::from_utf8_lossy(&bytes);
            assert!(
                hay.contains("(1)"),
                "content stream missing the rendered page-number glyph run '(1)':\n{hay}"
            );
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

fn compile_page_footer_fixture() -> std::rc::Rc<satysfi_lang::value::DocumentValue> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/page-footer.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("page-footer fixture must compile")
}

/// End-to-end coverage for the Slice 1 real `page-break`
/// (docs/plans/document-page-model.md): a 40-paragraph body overflows a
/// 640pt content scheme onto a second A4Paper page, and a footer closure
/// renders `pbinfo#page-number` per page. Rendering each page on its own
/// (a 1-page slice of `doc.pages`) makes each page's footer glyph
/// unambiguous in its content stream — the load-bearing assertion that the
/// per-page loop re-`interp.apply`s the parts closure with an
/// *incremented* page number, not the same one twice.
#[test]
fn page_footer_fixture_overflows_to_two_pages_with_incrementing_footer_numbers() {
    let doc = compile_page_footer_fixture();
    assert_eq!(
        doc.pages.len(),
        2,
        "the 40-paragraph body must overflow onto exactly 2 pages, got {}",
        doc.pages.len()
    );

    // Media box: `A4Paper` is 595 x 842 pt (within 1pt of the ISO 216 mm
    // conversion), driven by the `page` argument, not the old default.
    assert!(
        (doc.geometry.paper_width.0 - 595.0).abs() < 1.0,
        "unexpected paper width: {}",
        doc.geometry.paper_width.0
    );
    assert!(
        (doc.geometry.paper_height.0 - 842.0).abs() < 1.0,
        "unexpected paper height: {}",
        doc.geometry.paper_height.0
    );

    let page1 = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages[0..1], &doc.images)
        .expect("page 1 must render");
    let page2 = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages[1..2], &doc.images)
        .expect("page 2 must render");
    let hay1 = String::from_utf8_lossy(&page1);
    let hay2 = String::from_utf8_lossy(&page2);

    assert!(
        hay1.contains("(1)"),
        "page 1's content stream is missing its footer glyph '1':\n{hay1}"
    );
    assert!(
        !hay1.contains("(2)"),
        "page 1 must not show page 2's footer glyph:\n{hay1}"
    );
    assert!(
        hay2.contains("(2)"),
        "page 2's content stream is missing its footer glyph '2':\n{hay2}"
    );
    assert!(
        !hay2.contains("(1)"),
        "page 2 must not show page 1's footer glyph:\n{hay2}"
    );
}
