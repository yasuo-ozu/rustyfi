//! SCRATCH HARNESS (not a regression test): measures, per lexical area,
//! which whitespace edits preserve the compiled output.
//!
//! Feeds `docs/plans/formatter-cst/ground-truth-whitespace.md`. Every case is
//! a pair of `.saty` sources that differ only in whitespace (or in a comment);
//! both are compiled through the real loader + evaluator + line breaker +
//! page breaker and compared on (a) the placed-box structure (`Debug` of
//! `doc.pages`, which is `PartialEq`) and (b) the rendered PDF bytes.
//!
//! Each case may carry a PROBE: a third source that must DIFFER from `a`.
//! That is the vacuity control — it proves the varying site actually reaches
//! the typeset output, so an `EQUAL` verdict means "the whitespace edit was
//! absorbed", not "nothing here was ever typeset".
//!
//!     RUSTFLAGS="-C linker-features=-lld" cargo test -p rustyfi \
//!         --test ws_ground_truth -- --ignored --nocapture

use std::path::{Path, PathBuf};

use rustyfi_pdf::{FontFlags, FontRegistry};

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

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

/// What a compile is compared on. `Err` carries the failure message, so a
/// case whose edit makes the file stop compiling is recorded rather than
/// panicking the whole sweep.
type Outcome = Result<(String, Vec<u8>, usize), String>;

fn compile(store: &rustyfi_pdf::TtfFontStore, tag: &str, src: &str) -> Outcome {
    let dir = std::env::temp_dir().join(format!("rustyfi-wsgt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{tag}.saty"));
    std::fs::write(&path, src).map_err(|e| e.to_string())?;
    let merged = load_and_merge(&path)?;
    let doc = rustyfi_lang::compile_document_cst(&merged, store).map_err(|e| format!("{e}"))?;
    let boxes: usize = doc
        .pages
        .iter()
        .map(|p| p.lines.iter().map(|l| l.contents.len()).sum::<usize>())
        .sum();
    let digest = format!("{:?}", doc.pages);
    let bytes = rustyfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, store, &doc.images)
        .map_err(|e| format!("render: {e}"))?;
    Ok((digest, bytes, boxes))
}

struct Case {
    id: &'static str,
    area: &'static str,
    edit: &'static str,
    a: String,
    b: String,
    /// Vacuity control: a source that MUST differ from `a`.
    probe: Option<String>,
}

/// `@require: stdja-mini` + a `document` wrapper. `prelude` goes between the
/// header and `document` (empty, or a `let … in` chain); `body` is the block
/// text inside `'< … >`.
fn doc_src(prelude: &str, body: &str) -> String {
    format!("@require: stdja-mini\n{prelude}document (|\n  title = {{T}};\n  author = {{A}};\n|) '<\n{body}\n>\n")
}

fn case(id: &'static str, area: &'static str, edit: &'static str, a: String, b: String) -> Case {
    Case { id, area, edit, a, b, probe: None }
}

fn case_p(
    id: &'static str,
    area: &'static str,
    edit: &'static str,
    a: String,
    b: String,
    probe: String,
) -> Case {
    Case { id, area, edit, a, b, probe: Some(probe) }
}

/// A plain Latin paragraph, and a CJK one. Both long enough to be typeset,
/// short enough to stay on one line so a structural diff is readable.
const LAT: &str = "Alpha beta gamma delta.";
const CJK: &str = "日本語";

fn cases() -> Vec<Case> {
    let mut v = Vec::new();

    // ---------------- PROGRAM AREA ----------------
    v.push(case_p(
        "P1",
        "program",
        "collapse spaces between program tokens (`let a = {X}` -> `let a={X}`)",
        doc_src("let a = {Hi}\nin\n", "+p { #a; there. }"),
        doc_src("let a={Hi}\nin\n", "+p { #a; there. }"),
        doc_src("let a = {HiX}\nin\n", "+p { #a; there. }"),
    ));
    v.push(case(
        "P2",
        "program",
        "space -> newline+indent between program tokens",
        doc_src("let a = {Hi}\nin\n", "+p { #a; there. }"),
        doc_src("let a =\n      {Hi}\nin\n", "+p { #a; there. }"),
    ));
    v.push(case(
        "P3",
        "program",
        "extra blank lines in the program area",
        doc_src("let a = {Hi}\nin\n", "+p { #a; there. }"),
        doc_src("\n\n\nlet a = {Hi}\n\n\n\nin\n\n", "+p { #a; there. }"),
    ));
    v.push(case(
        "P4",
        "program",
        "re-indent program lines (tabs and spaces)",
        doc_src("let a = {Hi}\nin\n", "+p { #a; there. }"),
        doc_src("\t\tlet a = {Hi}\n\t\t\tin\n", "+p { #a; there. }"),
    ));
    // stdja-mini's `document` (lib-rustyfi/dist/packages/stdja-mini.satyh:49)
    // ignores `title`/`author` entirely, so a record probe that edits them is
    // VACUOUS. Read a field of an own record into the typeset text instead.
    let rec_src = |r: &str| doc_src(&format!("let r = {r}\nlet x = r#a\nin\n"), "+p { #x; }");
    v.push(case_p(
        "P5",
        "program",
        "strip all whitespace inside a `(| … |)` record literal",
        rec_src("(| a = {Alpha}; b = {Beta} |)"),
        rec_src("(|a={Alpha};b={Beta}|)"),
        rec_src("(| a = {AlphaX}; b = {Beta} |)"),
    ));
    v.push(case(
        "P6",
        "program(active)",
        "whitespace between `+p` and its `{` argument",
        doc_src("", &format!("+p {{ {LAT} }}")),
        doc_src("", &format!("+p\n     {{ {LAT} }}")),
    ));
    v.push(case_p(
        "P7",
        "program(active)",
        "whitespace between `\\emph` and its `{` argument",
        doc_src("", "+p { x \\emph{y} z. }"),
        doc_src("", "+p { x \\emph\n   {y} z. }"),
        doc_src("", "+p { x \\emph{y} z! }"),
    ));
    v.push(case(
        "P8",
        "program(vert)",
        "whitespace between `'<` and the first `+p`",
        doc_src("", &format!("+p {{ {LAT} }}")),
        format!(
            "@require: stdja-mini\ndocument (|\n  title = {{T}};\n  author = {{A}};\n|) '<+p {{ {LAT} }}>\n"
        ),
    ));
    v.push(case_p(
        "P9",
        "program(vert)",
        "blank lines between two `+p`s",
        doc_src("", "+p { One. }\n+p { Two. }"),
        doc_src("", "+p { One. }\n\n\n\n   +p { Two. }"),
        doc_src("", "+p { One. }\n+p { Twoo. }"),
    ));
    v.push(case(
        "P10",
        "program(header)",
        "extra spaces after `@require:`",
        doc_src("", "+p { H. }"),
        doc_src("", "+p { H. }").replace("@require: ", "@require:      "),
    ));
    v.push(case(
        "P11",
        "program(header)",
        "TRAILING spaces on the `@require:` line",
        doc_src("", "+p { H. }"),
        doc_src("", "+p { H. }").replace("@require: stdja-mini\n", "@require: stdja-mini   \n"),
    ));
    v.push(case(
        "P12",
        "program(header)",
        "blank lines between the header and `document`",
        doc_src("", "+p { H. }"),
        doc_src("", "+p { H. }").replace("stdja-mini\n", "stdja-mini\n\n\n\n"),
    ));

    // ---------------- INLINE TEXT `{ }` ----------------
    v.push(case_p(
        "I1",
        "inline",
        "collapse a run of spaces: `{a   b}` -> `{a b}`",
        doc_src("", "+p { Alpha beta gamma. }"),
        doc_src("", "+p { Alpha     beta gamma. }"),
        doc_src("", "+p { Alphabeta gamma. }"),
    ));
    v.push(case_p(
        "I2",
        "inline",
        "space -> newline (Latin): `{a b}` vs `{a\\nb}`",
        doc_src("", "+p { Alpha beta gamma. }"),
        doc_src("", "+p { Alpha\nbeta gamma. }"),
        doc_src("", "+p { Alphabeta gamma. }"),
    ));
    v.push(case(
        "I3",
        "inline",
        "newline+indent -> newline (Latin continuation line re-indent)",
        doc_src("", "+p { Alpha\nbeta gamma. }"),
        doc_src("", "+p { Alpha\n        beta gamma. }"),
    ));
    v.push(case(
        "I4",
        "inline",
        "space-then-newline vs newline-then-space (Latin) — the lexer's \
         Space/Break decision keys on the run's FIRST char",
        doc_src("", "+p { Alpha \nbeta gamma. }"),
        doc_src("", "+p { Alpha\n beta gamma. }"),
    ));
    v.push(case_p(
        "I5",
        "inline",
        "leading whitespace inside `{ }`",
        doc_src("", &format!("+p {{{LAT} }}")),
        doc_src("", &format!("+p {{     {LAT} }}")),
        doc_src("", &format!("+p {{X{LAT} }}")),
    ));
    v.push(case(
        "I6",
        "inline",
        "leading newline + indent inside `{ }`",
        doc_src("", &format!("+p {{{LAT} }}")),
        doc_src("", &format!("+p {{\n      {LAT} }}")),
    ));
    v.push(case_p(
        "I7",
        "inline",
        "trailing whitespace / newline before `}`",
        doc_src("", &format!("+p {{ {LAT}}}")),
        doc_src("", &format!("+p {{ {LAT}   \n   }}")),
        doc_src("", &format!("+p {{ {LAT}X}}")),
    ));
    v.push(case(
        "I8",
        "inline",
        "CJK: single space between two CJK chars vs no space",
        doc_src("", &format!("+p {{ {CJK}{CJK} }}")),
        doc_src("", &format!("+p {{ {CJK} {CJK} }}")),
    ));
    v.push(case(
        "I9",
        "inline",
        "CJK: single SPACE vs single NEWLINE between two CJK chars",
        doc_src("", &format!("+p {{ {CJK} {CJK} }}")),
        doc_src("", &format!("+p {{ {CJK}\n{CJK} }}")),
    ));
    v.push(case(
        "I10",
        "inline",
        "CJK: one space vs a RUN of spaces between two CJK chars",
        doc_src("", &format!("+p {{ {CJK} {CJK} }}")),
        doc_src("", &format!("+p {{ {CJK}     {CJK} }}")),
    ));
    v.push(case(
        "I11",
        "inline",
        "CJK/Latin boundary: space vs no space",
        doc_src("", &format!("+p {{ {CJK}abc }}")),
        doc_src("", &format!("+p {{ {CJK} abc }}")),
    ));
    v.push(case(
        "I12",
        "inline",
        "CJK/Latin boundary: space vs newline",
        doc_src("", &format!("+p {{ {CJK} abc }}")),
        doc_src("", &format!("+p {{ {CJK}\nabc }}")),
    ));
    v.push(case(
        "I13",
        "inline",
        "CONTROL: delete the space before `\\emph` (must differ)",
        doc_src("", "+p { x \\emph{y} z. }"),
        doc_src("", "+p { x\\emph{y} z. }"),
    ));
    v.push(case(
        "I14",
        "inline",
        "`%` comment abutting text: `{a% c\\nb}` vs `{ab}`",
        doc_src("", "+p { Alpha% a comment\nbeta gamma. }"),
        doc_src("", "+p { Alphabeta gamma. }"),
    ));
    v.push(case(
        "I15",
        "inline",
        "`%` comment abutting text vs the same with a space kept",
        doc_src("", "+p { Alpha% a comment\nbeta gamma. }"),
        doc_src("", "+p { Alpha beta gamma. }"),
    ));
    v.push(case(
        "I16",
        "inline",
        "`%` comment AFTER a space: `{a %c\\nb}` vs `{a b}`",
        doc_src("", "+p { Alpha %a comment\nbeta gamma. }"),
        doc_src("", "+p { Alpha beta gamma. }"),
    ));
    v.push(case(
        "I17",
        "inline",
        "comment BODY changed (program-invisible?)",
        doc_src("", "+p { Alpha %a comment\nbeta. }"),
        doc_src("", "+p { Alpha %an entirely different comment body!!\nbeta. }"),
    ));
    v.push(case(
        "I18",
        "inline",
        "whitespace INSIDE a nested `{ }` group argument",
        doc_src("", "+p { x \\emph{ y } z. }"),
        doc_src("", "+p { x \\emph{y} z. }"),
    ));
    v.push(case(
        "I19",
        "inline",
        "space before `}` of a nested group vs after it",
        doc_src("", "+p { x \\emph{y } z. }"),
        doc_src("", "+p { x \\emph{y}  z. }"),
    ));

    // ---------------- BLOCK TEXT `'< >` ----------------
    v.push(case_p(
        "B1",
        "block",
        "re-indent every `+p` line inside `'< >`",
        doc_src("", "+p { One. }\n+p { Two. }"),
        doc_src("", "        +p { One. }\n\t+p { Two. }"),
        doc_src("", "+p { One. }\n+p { Twoo. }"),
    ));
    v.push(case(
        "B2",
        "block",
        "join all block-level content onto one line",
        doc_src("", "+p { One. }\n+p { Two. }"),
        doc_src("", "+p { One. }+p { Two. }"),
    ));
    v.push(case(
        "B3",
        "block",
        "comment lines inside the block area",
        doc_src("", "+p { One. }\n+p { Two. }"),
        doc_src("", "% leading remark\n+p { One. }\n   % indented remark\n+p { Two. }\n%trailing"),
    ));
    v.push(case(
        "B4",
        "block",
        "whitespace immediately before `>`",
        doc_src("", "+p { One. }"),
        doc_src("", "+p { One. }\n\n\n      "),
    ));

    // ---------------- MATH `${ }` ----------------
    v.push(case_p(
        "M1",
        "math",
        "`${x   +   y}` vs `${x+y}`",
        doc_src("", "+p { m ${x+y} n. }"),
        doc_src("", "+p { m ${x   +   y} n. }"),
        doc_src("", "+p { m ${x+z} n. }"),
    ));
    v.push(case(
        "M2",
        "math",
        "newlines inside `${ }`",
        doc_src("", "+p { m ${x+y} n. }"),
        doc_src("", "+p { m ${x\n    +\n    y} n. }"),
    ));
    v.push(case(
        "M3",
        "math",
        "leading/trailing whitespace inside `${ }`",
        doc_src("", "+p { m ${x+y} n. }"),
        doc_src("", "+p { m ${   x+y   } n. }"),
    ));
    v.push(case_p(
        "M4",
        "math",
        "whitespace around `^` / `_`",
        doc_src("", "+p { m ${x^2} n. }"),
        doc_src("", "+p { m ${x ^ 2} n. }"),
        doc_src("", "+p { m ${x^3} n. }"),
    ));
    v.push(case(
        "M5",
        "math",
        "whitespace inside a nested `${ { } }` math group",
        doc_src("", "+p { m ${x^{2+3}} n. }"),
        doc_src("", "+p { m ${ x ^ { 2 + 3 } } n. }"),
    ));
    v.push(case(
        "M6",
        "math",
        "`%` comment inside `${ }`",
        doc_src("", "+p { m ${x+y} n. }"),
        doc_src("", "+p { m ${x %hi\n+y} n. }"),
    ));
    v.push(case(
        "M7",
        "math",
        "whitespace around a math area's OUTER delimiters (horizontal area)",
        doc_src("", "+p { m ${x+y} n. }"),
        doc_src("", "+p { m${x+y}n. }"),
    ));

    // ---------------- LITERALS ----------------
    // `embed-string : string -> inline-text`, so a program-area backtick
    // literal reaches the page as typeset text. `#a;` embeds it.
    let lit = |body: &str| doc_src(&format!("let a = embed-string {body}\nin\n"), "+p { #a; }");
    v.push(case(
        "L1",
        "literal(program)",
        "collapse spaces inside a program-area backtick literal",
        lit("`alpha  beta`"),
        lit("`alpha beta`"),
    ));
    v.push(case(
        "L2",
        "literal(program)",
        "UNIFORMLY re-indent every line of a multi-line literal",
        lit("`line one\n  line two`"),
        lit("`    line one\n      line two`"),
    ));
    v.push(case(
        "L3",
        "literal(program)",
        "re-indent ONE line of a multi-line literal (relative indent changed)",
        lit("`line one\n  line two`"),
        lit("`line one\n        line two`"),
    ));
    v.push(case(
        "L4",
        "literal(program)",
        "leading space in a plain literal: `` ` x` `` vs `` `x` `` (omit_pre on)",
        lit("` alpha`"),
        lit("`alpha`"),
    ));
    v.push(case(
        "L5",
        "literal(program)",
        "leading space in a `#`-prefixed literal (omit_pre OFF)",
        lit("#` alpha`"),
        lit("#`alpha`"),
    ));
    v.push(case(
        "L6",
        "literal(program)",
        "TRAILING space in a literal, `` `alpha ` `` vs `` `alpha` ``",
        lit("`alpha `"),
        lit("`alpha`"),
    ));
    v.push(case(
        "L7",
        "literal(program)",
        "trailing space kept by the closing `#`: `` `alpha `# `` vs `` `alpha`# ``",
        lit("`alpha `#"),
        lit("`alpha`#"),
    ));
    v.push(case(
        "L8",
        "literal(inline)",
        "collapse spaces inside an INLINE-text backtick literal (CodeText)",
        doc_src("", "+p { x `alpha  beta` y. }"),
        doc_src("", "+p { x `alpha beta` y. }"),
    ));

    // ---------------- inline-text separators: `|` and `*` ----------------
    // Both are lexed as `(space|break)* <marker>` followed by `skip_spaces`,
    // so the whitespace on either side of them should be absorbed.
    let itemize = |body: &str| {
        format!("@require: stdja-mini\n@require: itemize\ndocument (|\n  title = {{T}};\n  author = {{A}};\n|) '<\n{body}\n>\n")
    };
    v.push(case_p(
        "S1",
        "inline(item)",
        "whitespace around inline `*` itemize bullets",
        itemize("+p { \\listing{* one\n  * two\n} }"),
        itemize("+p { \\listing{*one*two} }"),
        itemize("+p { \\listing{* one\n  * twoo\n} }"),
    ));

    // ---------------- COMMENTS in the program area ----------------
    v.push(case(
        "C1",
        "program",
        "comment BODY changed in the program area",
        doc_src("let a = {Hi} % remark\nin\n", "+p { #a; }"),
        doc_src("let a = {Hi} % an utterly different remark\nin\n", "+p { #a; }"),
    ));
    v.push(case(
        "C2",
        "program",
        "trailing comment MOVED onto its own line",
        doc_src("let a = {Hi} % remark\nin\n", "+p { #a; }"),
        doc_src("% remark\nlet a = {Hi}\nin\n", "+p { #a; }"),
    ));
    v.push(case(
        "C3",
        "program",
        "comment re-indented",
        doc_src("  % remark\nlet a = {Hi}\nin\n", "+p { #a; }"),
        doc_src("\t\t\t% remark\nlet a = {Hi}\nin\n", "+p { #a; }"),
    ));
    // ---------------- active-area program arguments ----------------
    v.push(case_p(
        "P13",
        "program(active)",
        "whitespace between `\\cmd` and a `(`-parenthesised program argument",
        doc_src(
            "let-inline ctx \\tight s = read-inline ctx (embed-string s)\nin\n",
            "+p { x \\tight(`alpha`); y. }",
        ),
        doc_src(
            "let-inline ctx \\tight s = read-inline ctx (embed-string s)\nin\n",
            "+p { x \\tight\n     (`alpha`)\n   ; y. }",
        ),
        doc_src(
            "let-inline ctx \\tight s = read-inline ctx (embed-string s)\nin\n",
            "+p { x \\tight(`alphaX`); y. }",
        ),
    ));
    v.push(case(
        "P14",
        "program(header)",
        "`%` comment appended to a `@require:` line",
        doc_src("", "+p { H. }"),
        doc_src("", "+p { H. }").replace("stdja-mini\n", "stdja-mini % a remark\n"),
    ));

    // ---------------- inline text nested INSIDE math (`!{ … }`) ----------------
    let mathdoc = |body: &str| {
        format!("@require: stdja-mini\n@require: math\ndocument (|\n  title = {{T}};\n  author = {{A}};\n|) '<\n{body}\n>\n")
    };
    v.push(case_p(
        "M8",
        "math->inline",
        "collapse a space run inside math's `!{ … }` inline-text escape",
        mathdoc("+p { m ${\\text!{alpha beta}} n. }"),
        mathdoc("+p { m ${\\text!{alpha     beta}} n. }"),
        mathdoc("+p { m ${\\text!{alpha betaX}} n. }"),
    ));
    v.push(case(
        "M9",
        "math->inline",
        "CONTROL: delete the space inside math's `!{ … }` (must differ)",
        mathdoc("+p { m ${\\text!{alpha beta}} n. }"),
        mathdoc("+p { m ${\\text!{alphabeta}} n. }"),
    ));
    v.push(case(
        "M10",
        "math",
        "whitespace between a math command and its `!{` argument",
        mathdoc("+p { m ${\\text!{alpha}} n. }"),
        mathdoc("+p { m ${\\text  \n  !{alpha}} n. }"),
    ));

    // ---------------- literal VALUES, observed rather than typeset -----------
    // L1-L6 compare the RENDERED page, where `text_to_boxes` collapses
    // consecutive elastic spaces on its own (primitives.rs:2150-2160) and so
    // hides a difference in the string VALUE. These read the value back with
    // `string-length` and typeset the number, so the value itself is measured.
    let slen = |body: &str| {
        doc_src(
            &format!("let a = embed-string (arabic (string-length {body}))\nin\n"),
            "+p { #a; }",
        )
    };
    v.push(case_p(
        "V1",
        "literal(value)",
        "VALUE: collapse spaces inside a program literal (`string-length`)",
        slen("`alpha  beta`"),
        slen("`alpha beta`"),
        slen("`alpha  betaX`"),
    ));
    v.push(case(
        "V2",
        "literal(value)",
        "VALUE: uniformly re-indent every line of a multi-line literal",
        slen("`line one\n  line two`"),
        slen("`    line one\n      line two`"),
    ));
    v.push(case(
        "V3",
        "literal(value)",
        "VALUE: re-indent ONE line of a multi-line literal",
        slen("`line one\n  line two`"),
        slen("`line one\n        line two`"),
    ));
    v.push(case(
        "V4",
        "literal(value)",
        "VALUE: leading space in a `#`-prefixed literal (omit_pre OFF)",
        slen("#` alpha`"),
        slen("#`alpha`"),
    ));
    v.push(case(
        "V5",
        "literal(value)",
        "VALUE: trailing space in a plain literal (omit_post ON)",
        slen("`alpha `"),
        slen("`alpha`"),
    ));
    v.push(case(
        "V6",
        "literal(value)",
        "VALUE: trailing space kept by a closing `#`",
        slen("`alpha `#"),
        slen("`alpha`#"),
    ));
    v.push(case(
        "V7",
        "literal(value)",
        "VALUE: trailing NEWLINE in a plain literal",
        slen("`alpha\n`"),
        slen("`alpha`"),
    ));

    // ---------------- rigid vs elastic spaces in a STRING -------------------
    // The renderer's run collapsing is conditional: it is skipped for a RIGID
    // space (shrink == stretch == 0), primitives.rs:2146-2160. So a double
    // space in a string survives in a verbatim/code context.
    let ratio = |r: &str, body: &str| {
        doc_src(
            &format!(
                "let-inline ctx \\sp s = read-inline (ctx |> set-space-ratio 0.33 {r}) (embed-string s)\nin\n"
            ),
            body,
        )
    };
    v.push(case_p(
        "R1",
        "literal(render)",
        "RIGID space context: double space in a string vs single",
        ratio("0. 0.", "+p { x \\sp(`alpha  beta`); y. }"),
        ratio("0. 0.", "+p { x \\sp(`alpha beta`); y. }"),
        ratio("0. 0.", "+p { x \\sp(`alpha  betaX`); y. }"),
    ));
    v.push(case(
        "R2",
        "literal(render)",
        "ELASTIC space context: double space in a string vs single",
        ratio("0.1 0.2", "+p { x \\sp(`alpha  beta`); y. }"),
        ratio("0.1 0.2", "+p { x \\sp(`alpha beta`); y. }"),
    ));
    v.push(case(
        "R3",
        "cjk(string)",
        "TWO spaces vs ONE between two CJK chars, inside a STRING",
        doc_src("let a = embed-string `日本  語`\nin\n", "+p { #a; }"),
        doc_src("let a = embed-string `日本 語`\nin\n", "+p { #a; }"),
    ));

    // ---------------- round 3: the traps a formatter would actually hit ------
    v.push(case(
        "I20",
        "inline",
        "TAB vs space inside inline text",
        doc_src("", "+p { Alpha\tbeta. }"),
        doc_src("", "+p { Alpha beta. }"),
    ));
    v.push(case(
        "I21",
        "inline",
        "CRLF vs LF inside inline text",
        doc_src("", "+p { Alpha\r\nbeta. }"),
        doc_src("", "+p { Alpha\nbeta. }"),
    ));
    v.push(case(
        "I22",
        "inline",
        "escaped space `\\ ` (a Char token) vs a real space (a Space token)",
        doc_src("", "+p { Alpha\\ beta. }"),
        doc_src("", "+p { Alpha beta. }"),
    ));
    v.push(case(
        "I23",
        "inline",
        "three escaped spaces `\\ \\ \\ ` vs one",
        doc_src("", "+p { Alpha\\ \\ \\ beta. }"),
        doc_src("", "+p { Alpha\\ beta. }"),
    ));
    v.push(case_p(
        "I24",
        "inline",
        "CJK: RE-INDENT a continuation line (newline stays a newline)",
        doc_src("", &format!("+p {{ {CJK}\n{CJK} }}")),
        doc_src("", &format!("+p {{ {CJK}\n            {CJK} }}")),
        doc_src("", &format!("+p {{ {CJK}\n{CJK}{CJK} }}")),
    ));
    v.push(case(
        "I25",
        "inline",
        "CJK: TRIM TRAILING WHITESPACE at a line end inside `{ }`          (Space token -> Break token)",
        doc_src("", &format!("+p {{ {CJK} \n{CJK} }}")),
        doc_src("", &format!("+p {{ {CJK}\n{CJK} }}")),
    ));
    v.push(case(
        "I26",
        "inline",
        "Latin: TRIM TRAILING WHITESPACE at a line end inside `{ }`",
        doc_src("", "+p { Alpha \nbeta. }"),
        doc_src("", "+p { Alpha\nbeta. }"),
    ));
    v.push(case(
        "R4",
        "cjk(string)",
        "TWO spaces between CJK chars in a STRING vs NO space",
        doc_src("let a = embed-string `日本  語`\nin\n", "+p { #a; }"),
        doc_src("let a = embed-string `日本語`\nin\n", "+p { #a; }"),
    ));
    v.push(case(
        "V8",
        "literal(value)",
        "VALUE: uniform re-indent of a `#`-prefixed multi-line literal (omit_pre OFF,          so `min_indent_space`/`shave_indent` can act)",
        slen("#`line one\n  line two`"),
        slen("#`    line one\n      line two`"),
    ));
    v.push(case(
        "V9",
        "literal(value)",
        "VALUE: leading space in a PLAIN literal (omit_pre ON)",
        slen("` alpha`"),
        slen("`alpha`"),
    ));
    v.push(case(
        "V10",
        "literal(value)",
        "VALUE: a two-backtick literal ``…`` may contain a bare `` ` ``",
        slen("``a`b``"),
        slen("`ab`"),
    ));
    v.push(case_p(
        "P15",
        "program(active)",
        "whitespace inside a `(| … |)` record written in an ACTIVE area",
        doc_src(
            "let-inline ctx \\r rc = read-inline ctx rc#t\nin\n",
            "+p { x \\r(| t = {Alpha} |); y. }",
        ),
        doc_src(
            "let-inline ctx \\r rc = read-inline ctx rc#t\nin\n",
            "+p { x \\r(|t={Alpha}|); y. }",
        ),
        doc_src(
            "let-inline ctx \\r rc = read-inline ctx rc#t\nin\n",
            "+p { x \\r(| t = {AlphaX} |); y. }",
        ),
    ));
    v.push(case_p(
        "P16",
        "program(active)",
        "whitespace inside a `[ … ]` list written in an ACTIVE area",
        doc_src(
            "let-inline ctx \\l xs = match xs with | [] -> read-inline ctx {none} | (x :: _) -> read-inline ctx x\nin\n",
            "+p { x \\l[{Alpha}; {Beta}]; y. }",
        ),
        doc_src(
            "let-inline ctx \\l xs = match xs with | [] -> read-inline ctx {none} | (x :: _) -> read-inline ctx x\nin\n",
            "+p { x \\l[\n   {Alpha}\n ; {Beta}\n]; y. }",
        ),
        doc_src(
            "let-inline ctx \\l xs = match xs with | [] -> read-inline ctx {none} | (x :: _) -> read-inline ctx x\nin\n",
            "+p { x \\l[{AlphaX}; {Beta}]; y. }",
        ),
    ));

    // ---------------- round 4: re-wrapping a REAL paragraph ----------------
    // 40 words, long enough that the line breaker actually wraps it, so an
    // EQUAL verdict covers the line-breaking decisions and not just the glue.
    const PARA: &str = "A second paragraph long enough that greedy line breaking must wrap it onto several lines to prove that the box model, the glue justification, and the page layout all work together end to end.";
    let rewrap = |cols: usize| {
        let mut out = String::new();
        let mut col = 0usize;
        for w in PARA.split(' ') {
            if col == 0 {
                out.push_str(w);
                col = w.len();
            } else if col + 1 + w.len() > cols {
                out.push_str("\n      ");
                out.push_str(w);
                col = w.len();
            } else {
                out.push(' ');
                out.push_str(w);
                col += 1 + w.len();
            }
        }
        out
    };
    v.push(case_p(
        "I27",
        "inline",
        "RE-WRAP a real 40-word Latin paragraph (40 cols vs 70 cols vs one line)",
        doc_src("", &format!("+p {{ {} }}", rewrap(40))),
        doc_src("", &format!("+p {{ {PARA} }}")),
        doc_src("", &format!("+p {{ {} }}", rewrap(40).replace("second", "third"))),
    ));
    v.push(case(
        "I27b",
        "inline",
        "RE-WRAP the same paragraph at a different column",
        doc_src("", &format!("+p {{ {} }}", rewrap(40))),
        doc_src("", &format!("+p {{ {} }}", rewrap(70))),
    ));
    // A long CJK paragraph: re-wrapping it means inserting/removing NEWLINES.
    const JPARA: &str = "日本語の文章を組版するときには行分割の位置が重要になります。この段落は十分に長いので行分割が実際に起こります。";
    v.push(case_p(
        "I28",
        "inline",
        "RE-WRAP a real CJK paragraph by inserting NEWLINES only",
        doc_src("", &format!("+p {{ {JPARA} }}")),
        doc_src(
            "",
            &format!(
                "+p {{ {}\n      {}\n      {} }}",
                &JPARA.chars().take(20).collect::<String>(),
                &JPARA.chars().skip(20).take(20).collect::<String>(),
                &JPARA.chars().skip(40).collect::<String>()
            ),
        ),
        doc_src("", &format!("+p {{ {JPARA}。 }}")),
    ));
    v.push(case(
        "I29",
        "inline",
        "RE-WRAP the same CJK paragraph by inserting SPACES instead of newlines",
        doc_src("", &format!("+p {{ {JPARA} }}")),
        doc_src(
            "",
            &format!(
                "+p {{ {} {} {} }}",
                &JPARA.chars().take(20).collect::<String>(),
                &JPARA.chars().skip(20).take(20).collect::<String>(),
                &JPARA.chars().skip(40).collect::<String>()
            ),
        ),
    ));
    v.push(case(
        "I30",
        "inline",
        "MOVE an inline-text comment onto its own line (Latin)",
        doc_src("", "+p { Alpha %a remark\nbeta gamma. }"),
        doc_src("", "+p { Alpha\n%a remark\nbeta gamma. }"),
    ));
    v.push(case(
        "I31",
        "inline",
        "MOVE an inline-text comment onto its own line (CJK)",
        doc_src("", &format!("+p {{ {CJK} %a remark\n{CJK} }}")),
        doc_src("", &format!("+p {{ {CJK}\n%a remark\n{CJK} }}")),
    ));

    // localising I28
    v.push(case(
        "I32",
        "inline",
        "CJK: bare newline between two CJK runs vs no whitespace",
        doc_src("", "+p { 日本語日本語 }"),
        doc_src("", "+p { 日本語\n日本語 }"),
    ));
    v.push(case(
        "I33",
        "inline",
        "CJK: newline + 6 spaces between two CJK runs vs no whitespace",
        doc_src("", "+p { 日本語日本語 }"),
        doc_src("", "+p { 日本語\n      日本語 }"),
    ));
    v.push(case(
        "I34",
        "inline",
        "CJK long paragraph: bare newlines (no indent) vs one line",
        doc_src("", &format!("+p {{ {JPARA} }}")),
        doc_src(
            "",
            &format!(
                "+p {{ {}\n{}\n{} }}",
                &JPARA.chars().take(20).collect::<String>(),
                &JPARA.chars().skip(20).take(20).collect::<String>(),
                &JPARA.chars().skip(40).collect::<String>()
            ),
        ),
    ));
    v.push(case(
        "I35",
        "inline",
        "CJK long paragraph: ONE bare newline vs one line",
        doc_src("", &format!("+p {{ {JPARA} }}")),
        doc_src(
            "",
            &format!(
                "+p {{ {}\n{} }}",
                &JPARA.chars().take(20).collect::<String>(),
                &JPARA.chars().skip(20).collect::<String>()
            ),
        ),
    ));

    v.push(case(
        "C4",
        "program",
        "comment inserted BETWEEN two program tokens (where a space stood)",
        doc_src("let a = {Hi}\nin\n", "+p { #a; }"),
        doc_src("let a =% here\n{Hi}\nin\n", "+p { #a; }"),
    ));

    v
}

#[test]
#[ignore = "scratch measurement harness; run with --ignored --nocapture"]
fn measure_whitespace_sensitivity() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(run)
        .expect("spawn")
        .join()
        .expect("harness panicked");
}

fn run() {
    let registry = FontRegistry::discover(Some(&lib_root()), None, &FontFlags::default())
        .expect("font discovery")
        .expect("lib-rustyfi/dist/hash/fonts.satysfi-hash must exist (run download-fonts.sh)");
    let store = registry.build_store().expect("build_store");

    let cases = cases();
    let mut n_compiles = 0usize;
    let mut rows = Vec::new();
    for c in &cases {
        assert_ne!(c.a, c.b, "{}: the two sources must differ", c.id);
        let oa = compile(&store, &format!("{}a", c.id), &c.a);
        let ob = compile(&store, &format!("{}b", c.id), &c.b);
        n_compiles += 2;
        let verdict = match (&oa, &ob) {
            (Ok((da, pa, ba)), Ok((db, pb, bb))) => {
                let same_boxes = da == db;
                let same_pdf = pa == pb;
                if same_boxes && same_pdf {
                    "EQUAL".to_string()
                } else if same_boxes {
                    "PDF-ONLY-DIFF".to_string()
                } else {
                    format!("DIFFER (boxes {ba} vs {bb})")
                }
            }
            (Ok(_), Err(e)) => format!("B FAILED TO COMPILE: {e}"),
            (Err(e), Ok(_)) => format!("A FAILED TO COMPILE: {e}"),
            (Err(e1), Err(e2)) => format!("BOTH FAILED: {e1} / {e2}"),
        };
        let vacuity = match &c.probe {
            None => "-".to_string(),
            Some(p) => {
                let op = compile(&store, &format!("{}p", c.id), p);
                n_compiles += 1;
                match (&oa, &op) {
                    (Ok((da, _, _)), Ok((dp, _, _))) => {
                        if da == dp {
                            "PROBE-VACUOUS!!".to_string()
                        } else {
                            "probe-live".to_string()
                        }
                    }
                    (_, Err(e)) => format!("probe failed: {e}"),
                    (Err(_), _) => "a failed".to_string(),
                }
            }
        };
        // Where they differ structurally, show the first divergence so the
        // write-up can quote an exact difference rather than "not equal".
        let detail = match (&oa, &ob) {
            (Ok((da, _, _)), Ok((db, _, _))) if da != db => {
                let i = da
                    .bytes()
                    .zip(db.bytes())
                    .position(|(x, y)| x != y)
                    .unwrap_or(da.len().min(db.len()));
                let lo = i.saturating_sub(90);
                format!(
                    "\n      A@{i}: …{}\n      B@{i}: …{}",
                    &da[lo..(i + 90).min(da.len())],
                    &db[lo..(i + 90).min(db.len())]
                )
            }
            _ => String::new(),
        };
        println!(
            "{:<5} {:<16} {:<14} {:<8} {}{}",
            c.id, c.area, verdict, vacuity, c.edit, detail
        );
        rows.push((c.id, c.area, verdict, vacuity, c.edit));
    }
    println!("\n{} cases, {} compiles", cases.len(), n_compiles);
    println!("\n--- markdown ---");
    for (id, area, verdict, vacuity, edit) in rows {
        println!("| {id} | {area} | {edit} | {verdict} | {vacuity} |");
    }
}
