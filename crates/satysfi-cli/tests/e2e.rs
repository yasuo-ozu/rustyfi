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
fn as_v006(cst: satysfi_loader::LoadedCst) -> satysfi_syntax::cst::File {
    match cst {
        satysfi_loader::LoadedCst::V0_0_6(f) => f,
        satysfi_loader::LoadedCst::V0_1(_) => {
            unreachable!("this test's load_and_merge is the V0_0_6-only path")
        }
    }
}

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
    let entry_cst = as_v006(entry_file.cst);
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

/// `annot` (and its `@require` chain: pervasives, color, gr, option) parses
/// deeply enough to overflow the default 8 MiB test-thread stack — mirrors
/// `satysfi-lang/tests/stdlib_tier0.rs`'s helper of the same name.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
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
/// box via one `cm`, never per-coordinate). Also covers roadmap C1
/// (`draw-text`, real glyph emission): the same callback draws a real
/// `read-inline`d text run above the rectangle, and the content stream
/// must additionally carry that run's `Td`/`Tj` — end-to-end proof
/// `place_graphics`'s `NestedEmitter` reaches `render_pdf`'s own text path
/// through the full compile pipeline, not just a hand-built `Page`.
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

    // Roadmap C1: `draw-text (0pt, 25pt) (read-inline ctx {Hi})` inside the
    // same callback — a real inline text run, emitted via `place_graphics`'s
    // `NestedEmitter` re-entering the writer's own `emit_box` at the run's
    // box-local anchor. (Page count still 1: `compile_graphics_fixture`
    // already asserts `doc.pages.len() == 1` before rendering.)
    for op in ["0 25 Td", "(Hi) Tj"] {
        assert!(hay.contains(op), "content stream missing {op:?}:\n{hay}");
    }
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

// ============================================================================
// Group A: /Annots + /Dests + /Outlines emission (docs/plans/
// hooks-annotations-crossref.md §B/§C) — `annot-hook.saty` reaches them via
// a raw `hook-page-break` closure (no §D frame/deco firing needed);
// `href_fixture_*` below exercises the §D path (`inline-frame-breakable`)
// through the real `annot.satyh` `\href`.
// ============================================================================

fn compile_annot_hook_fixture() -> std::rc::Rc<satysfi_lang::value::DocumentValue> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annot-hook.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("annot-hook fixture must compile")
}

/// A raw `hook-page-break` closure registers a named destination
/// (`register-destination`) and a URI link (`register-link-to-uri`) on the
/// page it fires on; a top-level `register-outline` call registers a
/// 2-entry outline, one item keyed to the same destination. `render_pdf`
/// (the 3-arg wrapper) would silently drop all of this — this fixture must
/// go through `render_pdf_with(..., &doc.extras)`.
#[test]
fn annot_hook_fixture_emits_annots_dests_and_outlines() {
    let doc = compile_annot_hook_fixture();
    assert_eq!(doc.pages.len(), 1);
    let bytes = satysfi_pdf::render_pdf_with(&doc.geometry, &doc.pages, &doc.images, &doc.extras)
        .expect("PDF rendering must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
    let hay = String::from_utf8_lossy(&bytes);

    for needle in [
        "/Annots",
        "/Subtype /Link",
        "/URI (https://example.com/)",
        "/Dests",
        "/XYZ",
        "/Outlines",
        "/Title",
    ] {
        assert!(hay.contains(needle), "content missing {needle:?}:\n{hay}");
    }

    // The outline item keyed `top` and the /Dests destination it points at
    // must share the SAME minted name (`nameddest0` — the first key seen,
    // by `register-destination`, since it fires before `register-outline`
    // resolves the same key through the shared `dest_name` table).
    assert!(
        hay.contains("nameddest0"),
        "the outline item's /Dest and the /Dests key must agree on a shared name:\n{hay}"
    );
}

fn compile_href_fixture() -> std::rc::Rc<satysfi_lang::value::DocumentValue> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/href.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    satysfi_lang::compile_document_cst(&merged, &metrics).expect("href fixture must compile")
}

/// The real `annot.satyh` `\href` end-to-end: `inline-frame-breakable`
/// builds the atomic frame, `fire_hooks`/`fire_inline_frame` fires its
/// `decoS` (`link-to-uri-frame`) once placed, landing a real `/Annots`
/// entry with a non-degenerate rect (the frame's fitted content width).
#[test]
fn href_fixture_emits_a_real_link_annotation_with_a_nonzero_width_rect() {
    run_with_big_stack(|| {
        let doc = compile_href_fixture();
        let bytes =
            satysfi_pdf::render_pdf_with(&doc.geometry, &doc.pages, &doc.images, &doc.extras)
                .expect("PDF rendering must succeed");
        let hay = String::from_utf8_lossy(&bytes);
        assert!(hay.contains("/Annots"), "missing /Annots:\n{hay}");
        assert!(
            hay.contains("/URI (https://example.com/)"),
            "missing /URI:\n{hay}"
        );

        // Parse the four /Rect operands and assert a positive width — proof
        // the frame's fitted content ("click here") actually measured to
        // something, not a degenerate zero-size box.
        let idx = hay.find("/Rect").expect("missing /Rect array");
        let rest = &hay[idx + "/Rect".len()..];
        let open = rest.find('[').expect("/Rect must be followed by an array");
        let close = rest.find(']').unwrap();
        let nums: Vec<f64> = rest[open + 1..close]
            .split_whitespace()
            .map(|s| s.parse().expect("a /Rect operand must be a number"))
            .collect();
        assert_eq!(nums.len(), 4, "/Rect must have 4 operands: {nums:?}");
        let width = (nums[2] - nums[0]).abs();
        assert!(
            width > 0.0,
            "the link rect must have a positive width, got {nums:?}"
        );
    });
}

// ============================================================================
// Group B (docs/plans/document-page-model.md §C, item #5): the real
// `add-footnote` float accumulator. F1 below is the data-loss regression —
// it FAILS at HEAD (before this group's `chop_page` accumulator) because
// the footnote body text is silently dropped by the old STAND-IN.
// ============================================================================

fn compile_footnote_fixture() -> std::rc::Rc<satysfi_lang::value::DocumentValue> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/footnote.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("footnote fixture must compile")
}

/// `add-footnote`'s block ends up on the SAME page as its referencing line,
/// bottom-placed below it (`baseline_y` strictly greater than every body
/// line's), and its text actually reaches the rendered PDF's content
/// stream — the assertion that FAILS at HEAD today, when `add-footnote`
/// was a documented no-op that dropped the block.
#[test]
fn footnote_fixture_places_the_footnote_body_below_the_reference_and_renders_its_text() {
    let doc = compile_footnote_fixture();
    assert_eq!(
        doc.pages.len(),
        1,
        "the short body + footnote should fit on one page, got {}",
        doc.pages.len()
    );

    let lines = &doc.pages[0].lines;
    assert!(
        lines.len() >= 2,
        "expected at least the reference line and the footnote line, got {}",
        lines.len()
    );
    let max_baseline = lines
        .iter()
        .map(|l| l.baseline_y.0)
        .fold(f64::MIN, f64::max);
    let reference_baseline = lines[0].baseline_y.0;
    assert!(
        max_baseline > reference_baseline,
        "the bottom-placed footnote line's baseline ({max_baseline}) must sit \
         strictly below the reference line's ({reference_baseline})"
    );

    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images)
        .expect("PDF rendering must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let tmp =
        std::env::temp_dir().join(format!("satysfi-rust-e2e-footnote-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();
    let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();

    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.contains("Reference line with a footnote marker."),
                "pdftotext output missing the reference line:\n{text}"
            );
            assert!(
                text.contains("This is the distinctive footnote body text."),
                "pdftotext output missing the footnote body text — the data-loss \
                 regression this fixture guards against:\n{text}"
            );
        }
        _ => {
            let hay = String::from_utf8_lossy(&bytes);
            for expected in ["(Reference)", "(distinctive)", "(footnote)"] {
                assert!(
                    hay.contains(expected),
                    "content stream missing {expected:?} — the footnote body text \
                     must reach the rendered PDF:\n{hay}"
                );
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

// ============================================================================
// Group B (docs/plans/document-page-model.md §A, item #8): real multi-
// column `page-break-two-column` / `page-break-multicolumn`. F2 proves a
// genuine SECOND column at a shifted x-origin (not the old single-column
// stand-in); F3 proves `columnhookf` fires once per COLUMN (not once per
// page) by counting its marker line per page.
// ============================================================================

fn compile_twocolumn_fixture() -> std::rc::Rc<satysfi_lang::value::DocumentValue> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/twocolumn.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("twocolumn fixture must compile")
}

/// `page-break-two-column A4Paper 250pt …`: page 1 must carry lines at BOTH
/// the content scheme's own `text-origin.x` (72pt) and that plus the
/// 250pt shift (322pt) — real second-column geometry, not a same-x
/// single-column fallback.
#[test]
fn twocolumn_fixture_places_a_second_column_at_the_shifted_x_origin() {
    let doc = compile_twocolumn_fixture();
    assert!(!doc.pages.is_empty());

    let page1 = &doc.pages[0];
    let text_origin_x = 72.0_f64;
    let shifted_x = 72.0_f64 + 250.0;

    let has_col1 = page1
        .lines
        .iter()
        .any(|l| (l.x.0 - text_origin_x).abs() < 0.01);
    let has_col2 = page1
        .lines
        .iter()
        .any(|l| (l.x.0 - shifted_x).abs() < 0.01);
    assert!(
        has_col1,
        "page 1 should have lines at the first column's x = 72pt, got x values: {:?}",
        page1.lines.iter().map(|l| l.x.0).collect::<Vec<_>>()
    );
    assert!(
        has_col2,
        "page 1 should have lines at the second column's x = 322pt (72pt + 250pt shift), got x values: {:?}",
        page1.lines.iter().map(|l| l.x.0).collect::<Vec<_>>()
    );
}

fn compile_multicolumn_fixture() -> std::rc::Rc<satysfi_lang::value::DocumentValue> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multicolumn.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("multicolumn fixture must compile")
}

fn count_colmark_lines(page: &satysfi_backend::Page) -> usize {
    fn text_of(bx: &satysfi_backend::PureHorzBox, out: &mut String) {
        match bx {
            satysfi_backend::PureHorzBox::InnerString { text, .. } => out.push_str(text),
            satysfi_backend::PureHorzBox::Discretionary { no_break, .. } => {
                for b in no_break {
                    text_of(b, out);
                }
            }
            _ => {}
        }
    }
    page.lines
        .iter()
        .filter(|l| {
            let mut s = String::new();
            for (_, bx) in &l.contents {
                text_of(bx, &mut s);
            }
            s.contains("COLMARK")
        })
        .count()
}

/// `page-break-multicolumn A4Paper [250pt] …`: `columnhookf` fires at the
/// start of EVERY column (pageBreak.ml:700), so a full 2-column page must
/// carry exactly 2 `COLMARK` lines — one per column, not one per page.
#[test]
fn multicolumn_fixture_fires_the_column_hook_once_per_column() {
    let doc = compile_multicolumn_fixture();
    assert!(
        doc.pages.len() >= 2,
        "the 60-paragraph body over 300pt/column should span multiple pages, got {}",
        doc.pages.len()
    );
    // Every page but possibly the last (which may run out of content
    // mid-column) should show 2 COLMARK lines; the FIRST page is
    // guaranteed full content on both columns.
    let first_page_marks = count_colmark_lines(&doc.pages[0]);
    assert_eq!(
        first_page_marks, 2,
        "page 1 (guaranteed full) must carry exactly 2 COLMARK lines (one per column), got {first_page_marks}"
    );
}

// ============================================================================
// Capstone (build-order-to-stdja.md item #2): a real `@require: stdjabook`
// document rendered end-to-end through the FULL pipeline (loader -> elaborate
// -> typecheck -> eval -> line break -> page break -> PDF) to a PDF whose text
// `pdftotext` can extract. The document classes are already proven to
// load+evaluate (stdlib_tier0.rs); this closes the loop with the
// render_pdf -> pdftotext layer for a full document class. The body is Latin
// (WinAnsi, renders via base-14); CJK glyph rendering is gated on an
// embeddable TrueType CJK face the host lacks (see Group D / download-fonts.sh).
// ============================================================================

/// Locate a real regular TrueType face (DejaVu) for the capstone: the
/// stdjabook footer's em-dash page number is not in base-14, so a real font
/// is required — which also exercises Group D's `TtfFontStore` end-to-end.
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

#[test]
fn tier4_stdjabook_capstone_renders_to_extractable_text() {
    let font = match find_regular_ttf() {
        Some(p) => p,
        None => {
            eprintln!("skipping tier4 capstone: no DejaVu TrueType font found");
            return;
        }
    };
    run_with_big_stack(move || {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tier4.saty");
        let merged = load_and_merge(&fixture);
        let store = satysfi_pdf::TtfFontStore::load(&font, None, None)
            .expect("load DejaVu regular face");
        let doc = satysfi_lang::compile_document_cst(&merged, &store)
            .expect("tier4 stdjabook capstone must compile end-to-end");
        assert!(!doc.pages.is_empty(), "expected at least one page");
        assert!(
            doc.pages.iter().any(|p| !p.lines.is_empty()),
            "expected at least one non-empty page"
        );

        let bytes = satysfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, &store, &doc.images)
            .expect("PDF rendering must succeed");
        assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
        assert!(
            bytes.windows(9).any(|w| w == b"FontFile2"),
            "expected an embedded TrueType font (FontFile2) in the capstone PDF"
        );

        let tmp =
            std::env::temp_dir().join(format!("satysfi-rust-e2e-tier4-{}.pdf", std::process::id()));
        std::fs::write(&tmp, &bytes).unwrap();
        let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();
        match pdftotext {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                for word in ["quick", "brown", "fox"] {
                    assert!(
                        text.contains(word),
                        "pdftotext output missing {word:?} — the stdjabook capstone must \
                         render extractable Latin body text:\n{text}"
                    );
                }
            }
            _ => eprintln!(
                "pdftotext unavailable; the PDF-header + FontFile2-embed checks already passed"
            ),
        }
        let _ = std::fs::remove_file(&tmp);
    });
}

/// Slice 1 capstone (docs/plans/satysfi-0-1-0-support.md §3): a real
/// SATySFi 0.1 document — `val`-bound module library, comma records,
/// match…end, tuple page size — through the FULL spine (V0_1 lex -> cst_v1
/// parse -> loader -> v1 lowering -> shared elaborate/typecheck(V0_1)/eval
/// -> page break -> PDF) to pdftotext-extractable text. Base-14 only (the
/// fixture is all-WinAnsi, no capstone font dependency, so this never
/// skips like the tier4 capstone above can).
#[test]
fn v01_slice1_document_renders_to_extractable_text() {
    run_with_big_stack(move || {
        let entry = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v01-minimal.saty");
        let program = satysfi_loader::load(
            &entry,
            &satysfi_loader::LoadOptions {
                lib_root: Some(lib_root().join("dist-v01").join("packages")), // §4.3
                version: satysfi_syntax::SatysfiVersion::V0_1,
                // Legacy packaging (this 0.1 fixture uses `@require:`); the
                // Axis-B default. Spelled via `..Default::default()` so the
                // Ld3a `mode` field addition needs no explicit value here.
                ..Default::default()
            },
        )
        .expect("V0_1 must load once is_implemented() is flipped");
        assert_eq!(program.files.len(), 2, "v01-mini.satyh + v01-minimal.saty");
        assert!(matches!(program.files[0].cst, satysfi_loader::LoadedCst::V0_1(_)));

        let metrics = satysfi_pdf::Base14Metrics;
        let doc = satysfi_lang::compile_document_v1(&program.files, &metrics)
            .expect("the Slice-1 v0.1 capstone must compile end-to-end");
        assert_eq!(doc.pages.len(), 1);
        assert!(doc.pages[0].lines.len() >= 4);

        let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        // A4 via the v01 tuple page size: 210mm = 595.276pt.
        assert!((doc.geometry.paper_width.0 - 595.276).abs() < 0.01);

        let tmp = std::env::temp_dir().join(format!("satysfi-rust-e2e-v01-{}.pdf", std::process::id()));
        std::fs::write(&tmp, &bytes).unwrap();
        let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();
        match pdftotext {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                // match…end evaluated: Some 41 -> 41 + 1.
                assert!(text.contains("The answer is 42"), "missing match/…/end result:\n{text}");
                // \emph through the val-inline binding.
                assert!(text.contains("Emphasis"), "missing \\emph output:\n{text}");
                // The v01-mini footer (pbinfo#page-number via arabic).
                assert!(text.contains('1'), "missing footer page number:\n{text}");
                // Sub-slice 2b surface: `val rec sum-list`, `let mutable`/
                // `<-`, `if`/`even`, and `val (+++)` all through the third
                // `+p` paragraph.
                assert!(
                    text.contains("Sum 10 count 42 parity even label 7"),
                    "missing sub-slice 2b paragraph:\n{text}"
                );
            }
            _ => eprintln!("skipping text assertion; pdftotext unavailable"),
        }
        let _ = std::fs::remove_file(&tmp);
    });
}
