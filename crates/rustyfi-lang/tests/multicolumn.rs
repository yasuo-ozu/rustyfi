//! Real multi-column `page-break` (docs/plans/document-page-model.md §A,
//! item #8 of `docs/plans/build-order-to-stdja.md`): `page_break_core`'s
//! shared per-page loop — `columnhookf` firing at the start of EVERY
//! column, `columnendhookf` firing exactly once per page (before
//! `pagepartsf`), and the page-number-limit guard. Mirrors
//! `page_prims.rs`'s style (a local `\math` command, `compile_document`
//! directly — no `@require:` needed for any of these).

use rustyfi_backend::{FontKey, FontMetrics, Length, PlacedLine, PureHorzBox};
use rustyfi_lang::{elaborate, primitives, typecheck, CompileError};

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

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    Ok(())
}

fn assert_well_typed(src: &str) {
    if let Err(e) = typecheck_str(src) {
        panic!("expected {src:?} to type-check, got error: {e}");
    }
}

fn assert_type_error(src: &str) {
    match typecheck_str(src) {
        Ok(()) => panic!("expected {src:?} to be rejected by the typechecker, but it passed"),
        Err(CompileError::Type(_)) => {}
        Err(other) => panic!("expected {src:?} to fail with a type error, got: {other}"),
    }
}

// A shared skeleton: a 200pt-wide context and a narrow (100pt) single-column
// content area — narrow enough that a handful of `Hello` lines overflow
// across pages, driving the per-page loop through multiple iterations.
const PREAMBLE: &str = "
let-inline ctx \\math m = inline-nil
in
let ctx = get-initial-context 200pt (command \\math) in
let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
let parts pbinfo =
  (| header-origin = (0pt, 0pt); header-content = block-nil;
     footer-origin = (0pt, 0pt); footer-content = block-nil |)
in
let-rec repeat n f =
  if n <= 0 then block-nil
  else (f n) +++ (repeat (n - 1) f)
in
";

/// Concatenate every `InnerString` glyph run's text reachable from one
/// placed line's contents, in order — a coarse but sufficient way to
/// recognize which source line ended up where (recurses into
/// `Discretionary::no_break`, the only composite this suite's fixtures
/// produce).
fn line_text(line: &PlacedLine) -> String {
    fn go(bx: &PureHorzBox, out: &mut String) {
        match bx {
            PureHorzBox::InnerString { text, .. } => out.push_str(text),
            PureHorzBox::Discretionary { no_break, .. } => {
                for b in no_break {
                    go(b, out);
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for (_, bx) in &line.contents {
        go(bx, &mut out);
    }
    out
}

// ============================================================================
// T10: `columnhookf` fires at the START of every column (here, one column
// per page since the shift list is `[]`) — its line must be the topmost
// (lowest `baseline_y`) real content on every page, and fire exactly once
// per page (one column per page in this fixture).
// ============================================================================

#[test]
fn columnhookf_prepends_its_line_to_the_top_of_every_page() {
    let mono = Mono;
    let src = format!(
        "{PREAMBLE}
         let columnhookf () = line-break true true ctx (read-inline ctx {{HOOK}}) in
         let columnendhookf () = block-nil in
         let body = repeat 12 (fun n -> line-break true true ctx (read-inline ctx {{Hello}})) in
         page-break-multicolumn A4Paper [] columnhookf columnendhookf content parts body"
    );
    let doc = rustyfi_lang::compile_document(&src, &mono)
        .expect("page-break-multicolumn with a columnhookf should compile and evaluate");
    assert!(
        doc.pages.len() >= 2,
        "the narrow content area should force at least 2 pages, got {}",
        doc.pages.len()
    );

    let mut hook_count = 0;
    for page in &doc.pages {
        assert!(
            !page.lines.is_empty(),
            "every page should have at least the hook line"
        );
        let topmost = page
            .lines
            .iter()
            .min_by(|a, b| a.baseline_y.0.partial_cmp(&b.baseline_y.0).unwrap())
            .unwrap();
        assert!(
            line_text(topmost).contains("HOOK"),
            "the topmost line of each page must be the columnhookf's own line, got {:?}",
            line_text(topmost)
        );
        hook_count += page
            .lines
            .iter()
            .filter(|l| line_text(l).contains("HOOK"))
            .count();
    }
    assert_eq!(
        hook_count,
        doc.pages.len(),
        "columnhookf must fire exactly once per page (one column per page here)"
    );
}

// ============================================================================
// T11: `columnendhookf` fires exactly once per page, AFTER the column loop
// and BEFORE `pagepartsf` — injecting content on its first call rolls that
// content onto a fresh page (the injected line opens page 2).
// ============================================================================

#[test]
fn columnendhookf_injected_content_opens_the_next_page() {
    let mono = Mono;
    let src = format!(
        "{PREAMBLE}
         let columnhookf () = block-nil in
         let-mutable fired <- false in
         let columnendhookf () =
           if !fired then block-nil
           else (fired <- true) before (line-break true true ctx (read-inline ctx {{INJECTED}}))
         in
         let body = line-break true true ctx (read-inline ctx {{Hello}}) in
         page-break-multicolumn A4Paper [] columnhookf columnendhookf content parts body"
    );
    let doc = rustyfi_lang::compile_document(&src, &mono)
        .expect("page-break-multicolumn with a columnendhookf should compile and evaluate");
    assert_eq!(
        doc.pages.len(),
        2,
        "the columnendhookf's one-shot injection must roll onto exactly 2 pages, got {}",
        doc.pages.len()
    );
    assert!(
        !line_text(&doc.pages[0].lines[0]).contains("INJECTED"),
        "page 1 must hold only the original body, not the injected line"
    );
    let page2_text = line_text(&doc.pages[1].lines[0]);
    assert!(
        page2_text.contains("INJECTED"),
        "page 2 must open with the columnendhookf's injected line, got {page2_text:?}"
    );
}

// ============================================================================
// T12: the page-number-limit guard. A `columnendhookf` that ALWAYS injects
// one more (trivially-fitting) line never lets `remaining` drain, so the
// shared loop must eventually refuse with an error mentioning the limit
// rather than looping forever.
// ============================================================================

#[test]
fn columnendhookf_that_always_injects_hits_the_page_number_limit() {
    let mono = Mono;
    let src = format!(
        "{PREAMBLE}
         let columnhookf () = block-nil in
         let columnendhookf () = line-break true true ctx (read-inline ctx {{X}}) in
         page-break-multicolumn A4Paper [] columnhookf columnendhookf content parts block-nil"
    );
    let err = rustyfi_lang::compile_document(&src, &mono)
        .expect_err("an always-injecting columnendhookf must never terminate on its own");
    assert!(
        err.to_string().to_lowercase().contains("page number limit"),
        "expected the page-number-limit guard's error message, got: {err}"
    );
}

// ============================================================================
// T13: `page-break-two-column` typechecks over its real 6-arg signature
// (`page -> length -> (unit -> block-boxes) -> pagecontf -> pagepartsf ->
// block-boxes -> document`), mirroring `page-break-multicolumn`'s existing
// typing coverage (page_prims.rs).
// ============================================================================

#[test]
fn page_break_two_column_typechecks_over_its_6_arg_signature() {
    assert_well_typed(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         let columnhookf () = block-nil in
         page-break-two-column A4Paper 250pt columnhookf content parts block-nil",
    );
}

#[test]
fn page_break_two_column_rejects_a_non_length_shift_argument() {
    assert_type_error(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         let columnhookf () = block-nil in
         page-break-two-column A4Paper 3 columnhookf content parts block-nil",
    );
}

#[test]
fn page_break_two_column_rejects_a_wrong_arity_call() {
    assert_type_error(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         page-break-two-column A4Paper 250pt content parts block-nil",
    );
}
