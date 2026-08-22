//! Probe documents for the cross-version import sweep, as data.
//!
//! Each case is one real Satyrographos package exercised twice: the `v01`
//! probe is a minimal **0.1** document that `@require:`s it against a lib root
//! holding only the 0.0.6 corpus (so every `@require:` genuinely crosses), and
//! the `v006` control exercises the same package from an ordinary 0.0.6
//! document — which separates a bridge failure from a pre-existing 0.0.6-side
//! gap.
//!
//! Transcribed byte for byte from `layout-tests/xver_sweep/{cases,v006}/*.saty`;
//! `expect_cross`/`expect_v006` mirror `layout-tests/xver_sweep_baseline.json`.
//! Keep entries alphabetical by `name`.

#![allow(dead_code)]

pub struct Case {
    /// Registry package name as the installer knows it, e.g. "code-printer".
    pub package: &'static str,
    /// Case name (the old fixture stem), e.g. "codeprinter".
    pub name: &'static str,
    /// The 0.1 crossing probe document.
    pub v01: &'static str,
    /// The 0.0.6 control document; `None` when the case has no control.
    pub v006: Option<&'static str>,
    /// Baseline: does the crossing case compile?
    pub expect_cross: bool,
    /// Baseline: does the 0.0.6 control compile?
    pub expect_v006: bool,
}

// Case stem and package name are not always the same word: `codeprinter` is
// the registry's `code-printer`, and `base` probes `base/string`. `mathpkg`'s
// `math` is the port's own bundled 0.0.6 corpus, NOT a registry install, so an
// installer driven off this field must skip it. `fss` is installed by
// `layout-tests/xver_sweep.py` as a dependency and has no case of its own.
pub const CASES: &[Case] = &[
    Case {
        package: "algorithm",
        name: "algorithm",
        v01: r#"@require: algorithm/algorithm
@import: h

let open H in
document (| title = `algorithm` |) '<
  +algorithmic<
    +p { a step }
  >
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: algorithm/algorithm

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +algorithmic<
    +p { a step }
  >
>
"#,
        ),
        expect_cross: false,
        expect_v006: true,
    },
    Case {
        package: "arrows",
        name: "arrows",
        v01: r#"@require: arrows/arrows
@import: h

let a = Arrow.solid (| line-width = 1.0, head-size = 1.0 |) in
let b = Arrow.invert a in
let open H in
document (| title = `arrows` |) '<
  +p { an arrow value was built. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: arrows/arrows

let a = Arrow.solid (| line-width = 1.0; head-size = 1.0 |) in
let b = Arrow.invert a in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { an arrow value was built. }
>
"#,
        ),
        expect_cross: false,
        expect_v006: false,
    },
    Case {
        package: "azmath",
        name: "azmath",
        v01: r#"@require: azmath/azmath
@import: h

let open H in
document (| title = `azmath` |) '<
  +p { azmath required. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: azmath/azmath

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { control. }
>
"#,
        ),
        expect_cross: false,
        expect_v006: true,
    },
    Case {
        package: "base",
        name: "base",
        v01: r#"@require: base/string
@import: h

let s = String.concat [`a`, `b`, `c`] in
let s-text = embed-string s in
let open H in
document (| title = `base` |) '<
  +p { base String.concat gives #s-text;. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: base/string

let s = String.concat [`a`; `b`; `c`] in
let s-text = embed-string s in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { base String.concat gives #s-text;. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "chemfml",
        name: "chemfml",
        v01: r#"@require: chemfml/chemfml
@import: h

let open H in
document (| title = `chemfml` |) '<
  +p { water is \chem(`H2O`);. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: chemfml/chemfml

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { water is \chem(`H2O`);. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "code-printer",
        name: "codeprinter",
        v01: r#"@require: code-printer/code-printer
@import: h

let open H in
document (| title = `code-printer` |) '<
  +code-printer(`let x = 1`);
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: code-printer/code-printer

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +code-printer(`let x = 1`);
>
"#,
        ),
        expect_cross: false,
        expect_v006: false,
    },
    Case {
        package: "colorbox",
        name: "colorbox",
        v01: r#"@require: colorbox/colorbox
@import: h

let opts = [Colorbox.colback Color.white, Colorbox.arc 3pt] in
let open H in
document (| title = `colorbox` |) '<
  +Colorbox.colorbox(opts)<
    +p { inside a 0.0.6 colorbox. }
  >
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: colorbox/colorbox

let opts = [Colorbox.colback Color.white; Colorbox.arc 3pt] in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +Colorbox.colorbox(opts)<
    +p { inside a 0.0.6 colorbox. }
  >
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "derive",
        name: "derive",
        v01: r#"@require: derive/derive
@import: h

let ast = DeriveDSL.derive ${A} in
let open H in
document (| title = `derive` |) '<
  +p { a derivation ast was built. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: derive/derive

let ast = DeriveDSL.derive ${A} in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { a derivation ast was built. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "easytable",
        name: "easytable",
        v01: r#"@require: easytable/easytable
@import: h

let fmt = [EasyTable.align-left, EasyTable.align-right] in
let rules = [EasyTable.toprule, EasyTable.bottomrule] in
let open H in
document (| title = `easytable` |) '<
  +p { a table follows. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: easytable/easytable

let fmt = [EasyTable.align-left; EasyTable.align-right] in
let rules = [EasyTable.toprule; EasyTable.bottomrule] in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { a table follows. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "enumitem",
        name: "enumitem",
        v01: r#"@require: enumitem/enumitem
@import: h

let open H in
document (| title = `enumitem` |) '<
  +itemize(EnumitemAlias.dot-arabic)<
    +item{first}<>
    +item{second}<>
  >
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: enumitem/enumitem

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +itemize(EnumitemAlias.dot-arabic)<
    +item{first}<>
    +item{second}<>
  >
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "figbox",
        name: "figbox",
        v01: r#"@require: figbox/figbox
@import: h

let fb = FigBox.frame 1pt Color.black (FigBox.dummy-box 40pt 20pt) in
let open H in
document (| title = `figbox` |) '<
  +fig-block(fb);
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: figbox/figbox

let fb = FigBox.frame 1pt Color.black (FigBox.dummy-box 40pt 20pt) in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +fig-block(fb);
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "latexcmds",
        name: "latexcmds",
        v01: r#"@require: latexcmds/latexcmds
@import: h

let open H in
document (| title = `latexcmds` |) '<
  +p { before\hspace(10pt);after }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: latexcmds/latexcmds

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { before\hspace(10pt);after }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "lipsum",
        name: "lipsum",
        v01: r#"@require: lipsum/lipsum
@import: h

let n = string-length Lipsum.quick-brown-fox-string in
let n-text = embed-string (arabic n) in
let open H in
document (| title = `lipsum` |) '<
  +p { fox string length is #n-text;. }
  +p { \quick-brown-fox; }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: lipsum/lipsum

let n = string-length Lipsum.quick-brown-fox-string in
let n-text = embed-string (arabic n) in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { fox string length is #n-text;. }
  +p { \quick-brown-fox; }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "math",
        name: "mathpkg",
        v01: r#"@require: math
@import: h

let open H in
document (| title = `mathpkg` |) '<
  +p { math package required. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: math

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { control. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "matrixcd",
        name: "matrixcd",
        v01: r#"@require: matrixcd/matrixcd
@import: h

let o = MatrixCD.row-sep 5pt in
let open H in
document (| title = `matrixcd` |) '<
  +p { matrixcd option built. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: matrixcd/matrixcd

let o = MatrixCD.row-sep 5pt in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { matrixcd option built. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "pagenumber",
        name: "pagenumber",
        v01: r#"@require: pagenumber/pagenumber
@import: h

let s = PageNumber.get-page-string 1 in
let s-text = embed-string s in
let open H in
document (| title = `pagenumber` |) '<
  +p { page string #s-text;. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: pagenumber/pagenumber

let s = PageNumber.get-page-string 1 in
let s-text = embed-string s in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { page string #s-text;. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "quotation",
        name: "quotation",
        v01: r#"@require: quotation/quotation
@import: h

let open H in
document (| title = `quotation` |) '<
  +quotation<
    +p { quoted text from a 0.0.6 package. }
  >
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: quotation/quotation

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +quotation<
    +p { quoted text from a 0.0.6 package. }
  >
>
"#,
        ),
        expect_cross: false,
        expect_v006: true,
    },
    Case {
        package: "railway",
        name: "railway",
        v01: r#"@require: railway/railway
@import: h

let r = Rail.push-line (10pt, 0pt) Rail.init in
let n = Rail.length r in
let n-text = embed-string (arabic n) in
let open H in
document (| title = `railway` |) '<
  +p { rail length #n-text;. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: railway/railway

let r = Rail.push-line (10pt, 0pt) Rail.init in
let n = Rail.length r in
let n-text = embed-string (arabic n) in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { rail length #n-text;. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "ruby",
        name: "ruby",
        v01: r#"@require: ruby/ruby
@import: h

let ib ctx = Ruby.ruby ctx [] [`kanji`] [{KANJI}] in
let open H in
document (| title = `ruby` |) '<
  +p { \ruby([`kanji`])([{KANJI}]); }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: ruby/ruby

let ib ctx = Ruby.ruby ctx [] [`kanji`] [{KANJI}] in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { \ruby([`kanji`])([{KANJI}]); }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "siunitx",
        name: "siunitx",
        v01: r#"@require: siunitx/siunitx
@import: h

let open H in
document (| title = `siunitx` |) '<
  +p { one \math(${\kilo\gram});. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: siunitx/siunitx

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { one ${\kilo\gram}. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "texlogo",
        name: "texlogo",
        v01: r#"@require: texlogo/texlogo
@import: h

let open H in
document (| title = `texlogo` |) '<
  +p { The \TeX; and \LaTeX; logos, typeset by a real 0.0.6 package. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: texlogo/texlogo

document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { The \TeX; and \LaTeX; logos, typeset by a real 0.0.6 package. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
    Case {
        package: "uline",
        name: "uline",
        v01: r#"@require: uline/uline
@import: h

let ds = Uline.make-uline 0.5pt Color.black in
let open H in
document (| title = `uline` |) '<
  +p { underlined via a 0.0.6 deco-set: \uline{hello}. }
>
"#,
        v006: Some(
            r#"@require: stdjabook
@require: uline/uline

let ds = Uline.make-uline 0.5pt Color.black in
document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<
  +p { underlined via a 0.0.6 deco-set: \uline{hello}. }
>
"#,
        ),
        expect_cross: true,
        expect_v006: true,
    },
];

/// The shared 0.1 helper every crossing probe `@import:`s, written beside the
/// entry document as `h.satyh`.
pub const HELPER_SATYH: &str = r#"% xver-capstone-helper.satyh — the minimal SATySFi 0.1 (dev-0-1-0) document-
% scaffolding module `xver-capstone.saty` needs (`document`/`+p`/`\math`),
% trimmed straight out of `v01-mini.satyh` (see that file's header comments
% for the full rationale of each surface form). Reached from the capstone
% entry via `@import:` (a same-directory sibling), NOT `@require:` — so it
% is never a `@require:`-resolved corpus target and stays `V0_1` under the
% loader's per-file version-detection rule (`design-cross-version-import.md`
% §5, Q4), exactly like `xver_import.rs`'s own `XVER_HELPER_SRC`. Keeping
% this scaffolding local (rather than requiring the real `v01-mini`/
% `stdja-mini` packages out of `dist-v01/packages/`) is what lets the
% capstone's `lib_root` point straight at `lib-rustyfi/` and reach the REAL
% frozen 0.0.6 corpus (`dist/packages/`) for its `@require:`s — see
% `xver_capstone.rs`'s module doc comment for the full lib-root writeup.

module H = struct

  val inline ctx \math m = embed-math ctx (read-math ctx m)

  val document record bt =
    let ctx = get-initial-context 440pt (command \math) in
    let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in
    let parts pbinfo =
      (| header-origin = (72pt, 72pt),  header-content = block-nil,
         footer-origin = (72pt, 800pt), footer-content =
           line-break true true ctx
             (inline-fil ++ (read-inline ctx (embed-string (arabic (pbinfo#page-number)))) ++ inline-fil) |)
    in
    page-break (210mm, 297mm) content parts (read-block ctx bt)

  val block ctx +p it =
    line-break true true ctx (read-inline ctx it ++ inline-fil)

end
"#;
