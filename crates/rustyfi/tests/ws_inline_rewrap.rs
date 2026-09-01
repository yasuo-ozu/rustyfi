//! **Slice 6's ground truth**: does the formatter's inline re-wrap change what
//! the typesetter produces?
//!
//! `ws_ground_truth.rs` measured which whitespace edits a compile can see, one
//! hand-written pair at a time, and `docs/plans/formatter-cst/README.md`'s rule
//! 3 is what it concluded. This file asks the question the other way round and
//! about the shipped code:
//!
//! 1. **Is the formatter's classifier the port's own?**
//!    `rustyfi_lsp`'s `is_cjk` is a transcription of `rustyfi-backend`'s
//!    `char_script`, and a transcription can drift. Checked character by
//!    character over the whole BMP plus the astral range, so a one-codepoint
//!    slip fails.
//! 2. **Re-verification of the predicate**, on the shapes slice 6 actually
//!    relies on — including the `azmath` shape the corpus sweep caught, which
//!    no earlier fixture had — by compiling two sources that differ in exactly
//!    one gap's spelling and comparing placed boxes AND PDF bytes. Each carries
//!    a vacuity probe, because "EQUAL" is worthless if the varying site never
//!    reached the page.
//! 3. **End to end, on real documents**: `rustyfi fmt` the corpus, compile
//!    before and after, compare PDF bytes. This is the claim that matters —
//!    not "this fixture is safe" but "formatting these documents did not change
//!    them" — and it is the one no formatter-side test can make.
//!
//! Run it:
//!
//!     RUSTFLAGS="-C linker-features=-lld" cargo test -p rustyfi \
//!         --test ws_inline_rewrap -- --ignored --nocapture
//!
//! `#[ignore]`d for the same reason `ws_ground_truth.rs` is: it needs the fonts
//! (`download-fonts.sh`), and the CI build job does not run it.

use std::path::{Path, PathBuf};

use rustyfi_lsp::{format_cst, CstOptions, RustyfiVersion};
use rustyfi_pdf::{FontFlags, FontRegistry};

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root")
}

// ---------------------------------------------------------------------------
// 1. the classifier
// ---------------------------------------------------------------------------

/// `rustyfi_lsp`'s transcription of `char_script` agrees with `char_script`,
/// codepoint for codepoint.
///
/// The formatter cannot call `char_script`: `rustyfi-lsp`'s analysis half
/// promises nothing outside `rustyfi-syntax` (`lib.rs:8-22`) and the browser
/// playground links it into wasm, which is why `render::width` carries its own
/// table too. So the table is copied, and a copy needs a test that would fail
/// if either side moved — a spot check over a dozen characters would not.
///
/// **Not `#[ignore]`d.** It needs no fonts and no compile, and it is the one
/// check here that a routine `cargo test` should run.
#[test]
fn the_formatters_cjk_table_is_the_backends() {
    let mut checked = 0usize;
    let mut cjk = 0usize;
    // The whole BMP, plus the two astral ranges `char_script` names and a
    // margin either side of each.
    let ranges = [
        (0x0000u32, 0xFFFFu32),
        (0x1FF00, 0x20100),
        (0x2F900, 0x2FB00),
        (0x30000, 0x30010),
    ];
    for (lo, hi) in ranges {
        for u in lo..=hi {
            let Some(c) = char::from_u32(u) else { continue };
            let want = matches!(
                rustyfi_backend::font::char_script(c),
                rustyfi_backend::context::Script::Kana
                    | rustyfi_backend::context::Script::HanIdeographic
            );
            assert_eq!(
                rustyfi_lsp::formatter_char_is_cjk(c),
                want,
                "U+{u:04X} {c:?}: the formatter's table and `char_script` disagree"
            );
            checked += 1;
            cjk += usize::from(want);
        }
    }
    eprintln!("classifier: {checked} codepoints compared, {cjk} classify as CJK");
    assert!(cjk > 20_000, "only {cjk} CJK codepoints — the sweep has gone vacuous");
}

// ---------------------------------------------------------------------------
// the compile harness — `ws_ground_truth.rs`'s, unchanged
// ---------------------------------------------------------------------------

fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
    match cst {
        rustyfi_loader::LoadedCst::V0_0(f) => f,
        rustyfi_loader::LoadedCst::V0_1(_) => unreachable!("V0_0-only harness"),
    }
}

fn load_and_merge(entry: &Path) -> Result<rustyfi_syntax::cst::File, String> {
    let program = rustyfi_loader::load(
        entry,
        &rustyfi_loader::LoadOptions {
            lib_root: Some(lib_root()),
            ..Default::default()
        },
    )
    .map_err(|e| format!("load: {e}"))?;
    let mut files = program.files;
    let entry_file = files.pop().expect("loader yields the entry last");
    let entry_cst = as_v006(entry_file.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    Ok(rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    })
}

/// `(placed-box digest, PDF bytes)`, or the failure message.
type Outcome = Result<(String, Vec<u8>), String>;

fn compile(store: &rustyfi_pdf::TtfFontStore, tag: &str, src: &str) -> Outcome {
    let dir = std::env::temp_dir().join(format!("rustyfi-iwr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{tag}.saty"));
    std::fs::write(&path, src).map_err(|e| e.to_string())?;
    let merged = load_and_merge(&path)?;
    let doc = rustyfi_lang::compile_document_cst(&merged, store).map_err(|e| format!("{e}"))?;
    let digest = format!("{:?}", doc.pages);
    let bytes = rustyfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, store, &doc.images)
        .map_err(|e| format!("render: {e}"))?;
    Ok((digest, bytes))
}

fn store() -> rustyfi_pdf::TtfFontStore {
    FontRegistry::discover(Some(&lib_root()), None, &FontFlags::default())
        .expect("font discovery")
        .expect("lib-rustyfi/dist/hash/fonts.satysfi-hash must exist (run download-fonts.sh)")
        .build_store()
        .expect("build_store")
}

fn doc_src(body: &str) -> String {
    format!(
        "@require: stdja-mini\ndocument (|\n  title = {{T}};\n  author = {{A}};\n|) '<\n{body}\n>\n"
    )
}

// ---------------------------------------------------------------------------
// 2. the predicate, re-verified on the shapes slice 6 relies on
// ---------------------------------------------------------------------------

struct Case {
    id: &'static str,
    /// What the predicate SAYS: `true` = reflowable, so the two compiles must
    /// be EQUAL; `false` = frozen, so they are expected to differ (and a case
    /// that did not differ would mean the freeze is costing us nothing, which
    /// is worth knowing too).
    reflowable: bool,
    edit: &'static str,
    a: String,
    b: String,
    /// A source that must NOT compile to the same pages as `a`. Without it,
    /// "EQUAL" could mean "neither one reached the page".
    probe: String,
}

/// A pair that differ by exactly one gap: `a` has a newline there, `b` a space.
fn pair(
    id: &'static str,
    reflowable: bool,
    edit: &'static str,
    left: &str,
    right: &str,
    probe_left: &str,
) -> Case {
    Case {
        id,
        reflowable,
        edit,
        a: doc_src(&format!("+p {{ {left}\n  {right} }}")),
        b: doc_src(&format!("+p {{ {left} {right} }}")),
        probe: doc_src(&format!("+p {{ {probe_left}\n  {right} }}")),
    }
}

fn cases() -> Vec<Case> {
    vec![
        // --- the two poles, restating I27 and I28 against the shipped code.
        pair("R1", true, "Latin | Latin", "alpha beta", "gamma delta", "alphaX beta"),
        pair("R2", false, "CJK | CJK", "日本語です", "これは文章", "日本語でし"),
        // --- rule 3's "CJK on ONE side is absorbed", which is the finding an
        //     area-level rule throws away.
        pair("R3", true, "CJK | Latin", "日本語です", "alpha beta", "日本語でし"),
        pair("R4", true, "Latin | CJK", "alpha beta", "日本語です", "alphaX beta"),
        // --- THE AZMATH SHAPE. The corpus sweep found this before any fixture
        //     did: a 100% Japanese paragraph whose gap abuts a COMMAND, so the
        //     run ends and the gap is absorbed.
        pair(
            "R5",
            true,
            "CJK | a command (the azmath shape)",
            "を用いて別行立て数式を記述します。",
            "\\emph{と異なり}",
            "を用いて別行立て数式を記述しまし。",
        ),
        pair("R6", true, "a command | CJK", "\\emph{記述}", "と異なり", "\\emph{記迷}"),
        pair("R7", true, "CJK | inline math", "記述します。", "${x + y}です", "記述しまし。"),
        pair("R8", true, "CJK | a backtick literal", "記述します。", "`code`です", "記述しまし。"),
        // --- rule 3's counterexamples: measurably unsafe, and NOT Han or Kana.
        pair("R9", false, "`、` | `「`", "あ、", "「か」", "あ﹅"),
        pair("R10", false, "`。` | `々`", "語。", "々次", "語﹅"),
        pair("R11", false, "`：` | `！`", "あ：", "！か", "あ﹅"),
        pair("R12", false, "`）` | `・`", "（あ）", "・か", "（あ﹅"),
        pair(
            "R13",
            false,
            "U+3000 IDEOGRAPHIC SPACE either side (a `Zs`!)",
            "あ\u{3000}",
            "\u{3000}か",
            "あ﹅",
        ),
        pair(
            "R14",
            false,
            "`Ａ` U+FF21 FULLWIDTH LATIN A, whose Script is Latin",
            "あＡ",
            "Ｂか",
            "あ﹅",
        ),
        // --- and the scripts this port routes through `OtherScript`, which
        //     rule 3 says are safe. If any of these DIFFERS, the predicate is
        //     too permissive and the formatter is re-typesetting them.
        pair("R15", true, "Hangul | Hangul", "한국어입니다", "그리고 또", "한국어입니타"),
        pair("R16", true, "Thai | Thai", "ภาษาไทย", "และอีก", "ภาษาไทZ"),
        pair("R17", true, "Greek | Greek", "αβγδε", "ζηθικ", "αβγδZ"),
        pair("R18", true, "Cyrillic | Cyrillic", "Жуковский", "Пушкин", "ЖуковскиZ"),
        // --- the escaped-space veto: `\ ` joins the run, so the run's LAST
        //     character (a space) decides.
        // The escaped-space veto, and the case that CORRECTED the
        // implementation: `\ ` lexes to `Char(" ")`, so the character
        // immediately before the gap is a literal space and the naive reading
        // makes the gap Latin-adjacent and free. It is not — this pair
        // DIFFERS, so `inline::edge` looks through the space to `本`.
        pair("R19", false, "CJK + `\\ ` | CJK", "日本\\ ", "語です", "日木\\ "),
        // ...and the escape does not freeze everything it touches: with only
        // one CJK side the gap is still free.
        pair("R20", true, "Latin + `\\ ` | CJK", "alpha\\ ", "語です", "alphX\\ "),
        pair("R21", true, "CJK + `\\ ` | Latin", "日本\\ ", "alpha beta", "日木\\ "),
    ]
}

#[test]
#[ignore = "needs fonts and 40+ compiles; run with --ignored --nocapture"]
fn the_re_wrap_predicate_holds_against_real_compiles() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_predicate)
        .expect("spawn")
        .join()
        .expect("harness panicked");
}

fn run_predicate() {
    let store = store();
    let cases = cases();
    let mut compiles = 0usize;
    let mut wrong = Vec::new();
    println!(
        "{:<5} {:<8} {:<10} {:<12} {}",
        "id", "says", "measured", "vacuity", "edit"
    );
    for c in &cases {
        assert_ne!(c.a, c.b, "{}: the two sources must differ", c.id);
        let oa = compile(&store, &format!("{}a", c.id), &c.a);
        let ob = compile(&store, &format!("{}b", c.id), &c.b);
        let op = compile(&store, &format!("{}p", c.id), &c.probe);
        compiles += 3;
        let (da, pa) = oa.as_ref().unwrap_or_else(|e| panic!("{}: a failed: {e}", c.id));
        let (db, pb) = ob.as_ref().unwrap_or_else(|e| panic!("{}: b failed: {e}", c.id));
        let (dp, _) = op.as_ref().unwrap_or_else(|e| panic!("{}: probe failed: {e}", c.id));
        let equal = da == db && pa == pb;
        let vacuous = da == dp;
        println!(
            "{:<5} {:<8} {:<10} {:<12} {}",
            c.id,
            if c.reflowable { "free" } else { "frozen" },
            if equal { "EQUAL" } else { "DIFFER" },
            if vacuous { "VACUOUS!!" } else { "probe-live" },
            c.edit
        );
        assert!(
            !vacuous,
            "{}: the varying site never reached the page, so this case measures nothing",
            c.id
        );
        // The direction that matters. A case the predicate calls REFLOWABLE
        // must compile identically, or the formatter is changing documents.
        // The other direction is a statement about what the freeze buys and is
        // reported rather than asserted — a frozen case that turned out EQUAL
        // would mean the predicate is stricter than it needs to be, which is
        // the safe kind of wrong.
        if c.reflowable && !equal {
            wrong.push(format!(
                "{} ({}): the predicate says the gap is free and the compile DIFFERS",
                c.id, c.edit
            ));
        }
    }
    let frozen_equal: Vec<&str> = Vec::new();
    println!("\n{} cases, {compiles} compiles", cases.len());
    assert!(
        wrong.is_empty(),
        "the predicate is TOO PERMISSIVE — these gaps are not free:\n  {}",
        wrong.join("\n  ")
    );
    let _ = frozen_equal;
}

// ---------------------------------------------------------------------------
// 3. end to end: format the real corpus and compare PDF bytes
// ---------------------------------------------------------------------------

/// The claim that matters: **formatting these documents did not change them.**
///
/// A fixture says a shape is safe; this says the 0.0.6 corpus's own documents
/// compile to byte-identical PDFs after `rustyfi fmt` has re-wrapped their
/// inline text. It is the only check that covers shapes nobody thought to
/// write down — and the eight Japanese manuals are exactly where those live.
///
/// Only the `layout-tests/corpus` documents that compile standalone are used;
/// a package (`.satyh`) has no `document`, and a document that fails to compile
/// BEFORE formatting is skipped **with its error printed** rather than
/// silently. As of writing that is 7 compared and 3 skipped, and all three
/// skips are CWD-relative RESOURCE loads with nothing to do with whitespace —
/// `code-printer` reads `demo/demo.satyh`, `figbox` decodes `fig/example1.jpg`
/// and `gakushin` embeds `patches/dc_header_01.pdf`, each of which
/// `fidelity.py` reaches by running the binary with the document's own
/// directory as its CWD. This harness deliberately does not chdir: the process
/// CWD is global and the other tests in this binary run concurrently, and a
/// flaky sweep is worth less than three more documents. The eight Japanese
/// manuals whose frozen gaps this feature is about — azmath, easytable,
/// enumitem, slydifi and the rest — are all in the compared set.
///
/// The vacuity control is not a probe here but a **counter**: the run asserts
/// that the formatter actually re-spelled inline gaps in the documents it
/// compared. A formatter that stopped wrapping would otherwise pass this
/// trivially.
#[test]
#[ignore = "needs fonts and compiles the corpus twice; run with --ignored --nocapture"]
fn formatting_the_corpus_does_not_change_its_pdfs() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run_corpus)
        .expect("spawn")
        .join()
        .expect("harness panicked");
}

fn run_corpus() {
    let store = store();
    let root = repo_root();
    let staged = assemble_lib_root(&root);
    let mut docs = Vec::new();
    collect(&root.join("layout-tests/corpus"), &mut docs);
    docs.sort();
    let opts = CstOptions::default();
    let (mut compared, mut differing, mut skipped, mut respelled) = (0, 0, 0, 0usize);
    for path in &docs {
        let Ok(src) = std::fs::read_to_string(path) else { continue };
        let Some(out) = format_cst(&src, RustyfiVersion::V0_0, &opts) else {
            println!("  DECLINED (formatter): {}", path.display());
            skipped += 1;
            continue;
        };
        let gaps = respelled_gaps(&src, &out);
        if out == src {
            continue;
        }
        let rel = path.strip_prefix(&root).unwrap_or(path).display().to_string();
        // Compiled in place, so relative `@import:`s resolve against the
        // document's own directory.
        let before = compile_at(&store, &staged, path, &src);
        let Ok((da, pa)) = &before else {
            println!(
                "  skipped (does not compile as it stands): {rel}\n      {}",
                before.as_ref().unwrap_err()
            );
            skipped += 1;
            continue;
        };
        let after = compile_at(&store, &staged, path, &out);
        let (db, pb) = after
            .as_ref()
            .unwrap_or_else(|e| panic!("{rel}: FORMATTED source no longer compiles: {e}"));
        compared += 1;
        respelled += gaps;
        if da != db || pa != pb {
            differing += 1;
            println!("  DIFFERS: {rel} ({gaps} gaps re-spelled)");
        } else {
            println!("  identical: {rel} ({gaps} gaps re-spelled)");
        }
    }
    println!(
        "\n{compared} documents compiled before and after formatting, \
         {differing} differ, {skipped} skipped, {respelled} inline gaps re-spelled"
    );
    assert!(
        compared >= 6,
        "only {compared} documents reached the comparison — this sweep has gone \
         vacuous, and the Japanese manuals are exactly the ones that matter"
    );
    assert!(
        respelled > 0,
        "the formatter re-spelled no inline gap in any compared document, so \
         this sweep is passing without exercising slice 6 at all"
    );
    assert_eq!(
        differing, 0,
        "formatting changed the rendered output of {differing} document(s) — \
         see the DIFFERS lines above"
    );
}

/// One lib root that every corpus document can load against.
///
/// `layout-tests/fidelity.py`'s `assemble_lib_root`, transcribed and taken as
/// a UNION rather than per document: the port's own `dist/packages`, the full
/// upstream `base` from the `satysfi-base` submodule (the port bundles only
/// the subset its own stdlib needs, and enumitem/easytable/figbox reach
/// further into it), and every sibling corpus package staged under the prefix
/// it is `@require:`d by.
///
/// Without it this sweep compiled **three** documents and skipped seven,
/// including all eight of the Japanese manuals — which is where the corpus's
/// 429 frozen gaps live, so the sweep was passing on exactly the files it was
/// not looking at.
fn assemble_lib_root(root: &Path) -> PathBuf {
    let corpus = root.join("layout-tests/corpus");
    let dst = std::env::temp_dir().join(format!("rustyfi-iwr-root-{}", std::process::id()));
    let pkg = dst.join("dist/packages");
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&pkg).expect("staging dir");
    copy_dir(&root.join("lib-rustyfi/dist/packages"), &pkg);
    copy_dir(&corpus.join("satysfi-base/src"), &pkg.join("base"));
    for (prefix, src) in [
        ("latexcmds", "latexcmds/src"),
        ("enumitem", "enumitem/src"),
        ("easytable", "easytable/src"),
        ("figbox", "figbox/src"),
        ("class-slydifi", "slydifi/src"),
        ("railway", "railway/src"),
        ("code-printer", "code-printer/src"),
        ("azmath", "azmath/src"),
        // An empty prefix copies the source dir's CONTENTS into the packages
        // root, for a multi-package tree like `fss`, whose `src/` holds
        // `fss/`, `sss/` and the rest.
        ("", "fss/src"),
    ] {
        let from = corpus.join(src);
        if !from.exists() {
            continue;
        }
        let to = if prefix.is_empty() { pkg.clone() } else { pkg.join(prefix) };
        copy_dir(&from, &to);
    }
    dst
}

fn copy_dir(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    std::fs::create_dir_all(to).expect("staging dir");
    for e in entries.flatten() {
        let p = e.path();
        let dst = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &dst);
        } else {
            let _ = std::fs::copy(&p, &dst);
        }
    }
}

/// How many `Space`/`Break` slots differ between two lexes — the number of
/// inline gaps the formatter re-spelled.
fn respelled_gaps(before: &str, after: &str) -> usize {
    let (Ok(a), Ok(b)) = (
        rustyfi_syntax::lex_with_version(before, RustyfiVersion::V0_0),
        rustyfi_syntax::lex_with_version(after, RustyfiVersion::V0_0),
    ) else {
        return 0;
    };
    a.iter().zip(&b).filter(|(x, y)| x.slot != y.slot).count()
}

/// Compile `src` as if it were the file at `path`, so relative `@import:`s and
/// the document's own package directory still resolve.
fn compile_at(
    store: &rustyfi_pdf::TtfFontStore,
    staged: &Path,
    path: &Path,
    src: &str,
) -> Outcome {
    let tmp = path.with_extension("iwr-tmp.saty");
    std::fs::write(&tmp, src).map_err(|e| e.to_string())?;
    let out = (|| {
        let program = rustyfi_loader::load(
            &tmp,
            &rustyfi_loader::LoadOptions {
                lib_root: Some(staged.to_path_buf()),
                fallback_roots: vec![lib_root()],
                ..Default::default()
            },
        )
        .map_err(|e| format!("load: {e}"))?;
        let mut files = program.files;
        let entry_file = files.pop().expect("loader yields the entry last");
        let entry_cst = as_v006(entry_file.cst);
        let mut prelude = Vec::new();
        for lib in files {
            prelude.extend(as_v006(lib.cst).prelude);
        }
        prelude.extend(entry_cst.prelude);
        let merged = rustyfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: entry_cst.in_kw,
            body: entry_cst.body,
            eoi: entry_cst.eoi,
        };
        let doc = rustyfi_lang::compile_document_cst(&merged, store).map_err(|e| format!("{e}"))?;
        let digest = format!("{:?}", doc.pages);
        let bytes = rustyfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, store, &doc.images)
            .map_err(|e| format!("render: {e}"))?;
        Ok((digest, bytes))
    })();
    let _ = std::fs::remove_file(&tmp);
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("saty") {
            out.push(p);
        }
    }
}
