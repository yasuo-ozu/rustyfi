//! `--mathml`: a math run written back as **MathML Core**, for a reader whose
//! browser lays out mathematics natively.
//!
//! ## Why MathML Core rather than MathML 3
//!
//! MathML 3 is a large language with `<mfenced>`, `<mlabeledtr>`, elementary
//! maths notation and fourteen values of `mathvariant`. **No shipping browser
//! implements it.** What Firefox has always had, Safari has had since 2013 and
//! Chromium since 109, is MathML Core — a deliberately smaller profile whose
//! layout is specified in CSS terms. Everything this module emits is in that
//! profile, and [`tests::every_element_is_mathml_core`] pins the element set
//! against the list rather than against what happens to render today.
//!
//! Two Core restrictions shape the output and are worth stating up front,
//! because both look like omissions:
//!
//! - **`mathvariant` has exactly one legal value in Core, `normal`, and only
//!   on `<mi>`.** So a bold or double-struck letter cannot be spelled as an
//!   attribute. It does not need to be: SATySFi writes the style into the
//!   CODEPOINT (`${\bold{R}}` is laid out as `𝐑` U+1D411, see
//!   `crate::latex::style_wrapper`), and a Mathematical Alphanumeric Symbol is
//!   what a MathML renderer wants anyway. The style survives here for free
//!   where `--katex` had to reconstruct a `\mathbb{…}` wrapper for it.
//! - **`<mi>` italicises a single Latin or Greek letter automatically**
//!   (`text-transform: math-auto` in Core's UA stylesheet). That would be
//!   WRONG for us, and [`identifier`] explains why: because the italic is in
//!   the codepoint, a plain ASCII `x` reaching a backend is a letter the
//!   document set UPRIGHT.
//!
//! ## What this can and cannot be
//!
//! It is the same RE-DERIVATION [`crate::latex`] is, from the same
//! [`crate::mathrec`] recovery — the `${…}` source is gone by the time a
//! backend runs, and there is no `\frac` node anywhere. So **MathML inherits
//! every loss `--katex` has**, and the README's table for that mode applies
//! here unchanged: a `math-paren` delimiter is a `Fill` path and vanishes, a
//! radical sign likewise, a script inside a fraction can invert, nested
//! fractions flatten, matrices and aligned environments come out as their
//! cells in `dx` order.
//!
//! **Nothing is invented to cover a loss.** No `<msqrt>`, `<mroot>`,
//! `<mtable>` or delimiter `<mo>` is ever synthesized: every element written
//! stands for something the recovery actually found. An `<msqrt>` guessed from
//! "there is a wide flat fill here" would render as a fact.
//!
//! **What is done instead is to MARK the loss** — see [`Approx`]. A run this
//! layer knows it did not render faithfully carries `class="rustyfi-approx"`
//! on its `<math>` element, so every equation where the PDF shows something
//! this mode does not is findable with one `grep`, per document, rather than
//! being a property of the corpus that has to be re-argued each time. Measured:
//! **6 of 48 equations in `latexcmds` and 47 of 329 in `azmath`.** The
//! alternative — degrading such a run to the Unicode mode's plain text — was
//! rejected on that measurement: it would demote correctly recovered fractions,
//! scripts and limits along with the parenthesis, which it cannot put back
//! either way.
//!
//! ## Two things this mode recovers that `--katex` cannot
//!
//! Both because MathML has an element for the shape and LaTeX's surface syntax
//! does not, not because the recovery got better:
//!
//! - **an accent attaches to its base** ([`Atom::Glyph`] records holding a lone
//!   combining mark). `crate::latex` has to emit `x\hat{}` — base and accent
//!   side by side — because it has no notion of "the atom before me" that
//!   survives its token stream. Here the previous element is in hand, so it
//!   becomes `<mover accent="true">`, which is what the PDF draws. **Only when
//!   the two are in one run**, which is a real restriction rather than a
//!   footnote: `azmath`'s `accent.satyh:52-55` builds every accent out of
//!   `math-graphics` and two `draw-text`s, so its base and its mark arrive as
//!   separate math BOXES and the pairing cannot see across them. Those fall
//!   back to `crate::latex`'s own answer, an accent over an empty group, and
//!   the run is marked approximate.
//! - **a limit stays a limit on any base.** `\limits` is legal in LaTeX only
//!   after a `\mathop`, so `crate::latex::is_big_operator` gates it behind a
//!   fixed list of fifteen commands and a centred script on anything else is
//!   silently demoted to a side script. `<munderover>` has no such rule, so the
//!   geometry [`crate::mathrec`] measured is emitted as measured.
//!
//! ## Delimiters are the CALLER's, as they are for `--katex`
//!
//! [`math_mathml`] returns the `<math>` element's CHILDREN and never the
//! element itself, because three decisions about that element belong to the
//! paragraph rather than to the box:
//!
//! - whether it says `display="block"`, which both backends settle at
//!   paragraph flush (`reflow::text::sole_math_ml`,
//!   `markdown::para::Para::display_math_ml`), where several math boxes'
//!   children are concatenated into ONE element — a formula is not one box;
//! - whether it says `class="rustyfi-approx"`, which is the OR of its boxes';
//! - whether it is written at all: a run holding nothing this layer can name
//!   must produce no element, not an empty `<math></math>` that reserves space
//!   and announces an equation that is not there.
//!
//! ## One line, in both formats and both display styles
//!
//! An element is never broken across lines, and in Markdown that is the
//! CommonMark constraint `crate::mathsvg::Wrap` documents: a Markdown
//! paragraph is one line, a renderer with `breaks: true` puts a `<br>` at every
//! newline including the ones inside an element, and a BLANK line would
//! terminate the HTML block outright and leave the rest of the equation as
//! literal text.
//!
//! `crate::mathsvg` answers that by pretty-printing a DISPLAYED drawing only,
//! which is its own block and so is safe. **Measured on the corpus, that
//! distinction does not reach the file**: `Para::render` runs
//! `collapse_spaces` over every non-code paragraph, which maps `\n` to a space
//! and collapses runs, so the five `Wrap::Block` drawings in `azmath`'s
//! Markdown output arrive as one line with single spaces between their
//! children — the same line the inline shape would have produced, plus the
//! spaces. So there is nothing here to inherit, and emitting newlines that are
//! converted to spaces a moment later would be worse than not emitting them:
//! MathML has no whitespace-insensitive layout to fall back on, and a
//! whitespace-only text node between two children of an `<mrow>` is content,
//! not formatting. One line, always.

use std::fmt::Write as _;

use rustyfi_backend::{GraphicsElem, MathGlyph};

use crate::mathrec::{self, Atom, Script};

/// Whether everything this run draws came out where the PDF puts it.
///
/// `Exact` means it did. `Approx` means the equation is still written — see
/// the module comment for why suppressing it would help nobody — but is known
/// not to match, for one of exactly two reasons this layer can detect:
///
/// - **ink that became nothing.** Every inked path in `rules` that did not
///   become an `<mfrac>` bar is a `math-paren` delimiter, a radical sign or an
///   `\overline`: there is no character to recover and nothing in MathML Core
///   that draws a path.
/// - **an accent with no base** ([`write_atoms`]). It is emitted over an empty
///   group, which renders beside the character it belongs on rather than over
///   it — legible and visibly approximate, which is the honest failure.
///
/// It is a returned VALUE rather than a class this module writes itself,
/// because the `<math>` element belongs to the caller (see the module comment)
/// and because a displayed equation made of several math boxes is `Approx` if
/// ANY of them is.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum Approx {
    Exact,
    /// The class the caller puts on `<math>`. One constant, because
    /// `reflow::text::sole_math_ml` reads it back off the open tag when it
    /// merges several boxes into one displayed equation.
    Approx,
}

impl Approx {
    /// `true` when at least one of `self` and `other` is [`Approx::Approx`] —
    /// how several boxes' verdicts combine into one equation's.
    pub(crate) fn or(self, other: Approx) -> Approx {
        if self == Approx::Approx || other == Approx::Approx {
            Approx::Approx
        } else {
            Approx::Exact
        }
    }
}

/// The class marking an equation whose drawing this layer could not fully
/// account for — see [`Approx`]. Named once because three places have to agree
/// about it exactly: [`open_tag`] writes it, `reflow::text::sole_math_ml`
/// reads it back, and the tests grep for it.
pub(crate) const APPROX_CLASS: &str = "rustyfi-approx";

/// The `<math>` open tag for one equation.
///
/// `xmlns` is not required by the HTML parser — an element named `math` is put
/// into the MathML namespace by the tokenizer itself — but it is required for
/// the document to be well-formed XML, which is how the output is validated,
/// and it costs 41 bytes once per equation. `crate::mathsvg`'s `<svg>` carries
/// its own for the same reason.
///
/// `display` is not decoration. In `display="block"` a browser sets
/// `math-style: normal`, which puts a big operator's limits above and below at
/// full size and sets a fraction at display proportions; inline it shrinks the
/// operator and moves the limits beside it. Getting it wrong turns every
/// displayed equation in a document into a cramped inline one — the same
/// distinction `\[…\]` versus `\(…\)` carries for `--katex`.
pub(crate) fn open_tag(display: bool, approx: Approx) -> String {
    let class = if approx == Approx::Approx {
        format!("math-ml {APPROX_CLASS}")
    } else {
        "math-ml".to_string()
    };
    format!(
        "<math xmlns=\"http://www.w3.org/1998/Math/MathML\" class=\"{class}\" \
         display=\"{}\">",
        if display { "block" } else { "inline" },
    )
}

/// The closing half of [`open_tag`]. A `<math>` is never nested inside another
/// one here, so the first occurrence of this in a rendered equation is
/// unambiguously its end — which is what lets `reflow::text::sole_math_ml`
/// find the body by scanning rather than by parsing.
pub(crate) const CLOSE_TAG: &str = "</math>";

/// One `PureHorzBox::Math` as the CHILDREN of a `<math>` element — see this
/// module's doc comment for why the element itself is the caller's, and why
/// the result is one line.
///
/// Empty when the run holds nothing this layer can name, in which case the
/// caller must write nothing at all.
pub(crate) fn math_mathml(glyphs: &[MathGlyph], rules: &[GraphicsElem]) -> (String, Approx) {
    let atoms = mathrec::recover(glyphs, rules);
    let mut detached = false;
    let body = write_atoms(&atoms, &mut detached);
    // Every inked path that did NOT become a fraction bar is something drawn
    // that has no MathML spelling. Counted rather than detected by kind: a
    // `math-paren` delimiter, a radical sign and an `\overline` are all
    // `Fill`s, and so is a bar, so the only thing that distinguishes them here
    // is whether the recovery used it.
    let approx = if detached || mathrec::inked_paths(rules) > count_fracs(&atoms) {
        Approx::Approx
    } else {
        Approx::Exact
    };
    (body, approx)
}

/// How many of the recovered atoms are fractions, counting a fraction inside a
/// fraction's half — which cannot happen today ([`crate::mathrec`]'s "one
/// level deep") but would make the [`Approx`] verdict wrong if it ever did.
fn count_fracs(atoms: &[Atom<'_>]) -> usize {
    atoms
        .iter()
        .map(|a| match a {
            Atom::Frac { above, below } => 1 + count_fracs(above) + count_fracs(below),
            _ => 0,
        })
        .sum()
}

/// Fold a recovered atom list into a `<math>` element's children, on one line.
///
/// The loop is not one-atom-at-a-time, for the reason `crate::latex`'s own
/// `write_atoms` gives: [`crate::mathrec`] hands back one atom per GLYPH
/// RECORD, so `\sum_{k=1}^{n}` arrives as a base and four separate script
/// atoms. MathML's script elements take exactly one script per position, so
/// emitting them individually would produce four nested `<msub>`s — which
/// renders as a staircase rather than as a limit.
///
/// `detached` is set when an accent had no base in this run to attach to; see
/// [`Approx`] and the accent arm below.
fn write_atoms(atoms: &[Atom<'_>], detached: &mut bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < atoms.len() {
        match &atoms[i] {
            // A leading gap carries no information, and a gap either side of a
            // `+` is spacing the operator dictionary will insert again — see
            // [`dictionary_spaces_it`].
            Atom::Space => {
                if !out.is_empty() && !dictionary_spaces_it(atoms, i) {
                    out.push_str("<mspace width=\"0.25em\"/>");
                }
                i += 1;
            }
            // A record holding nothing but combining marks is an ACCENT the
            // math layout placed over the character before it, not an atom of
            // its own. Attached to the previous element, which is the one
            // thing `crate::latex` cannot do; see this module's doc comment.
            //
            // **With nothing before it, the accent goes over an EMPTY group**
            // rather than being dropped, and the case is common rather than
            // pathological: `azmath`'s `accent.satyh:52-55` builds every accent
            // out of `math-graphics` and two `draw-text`s, so the base and the
            // mark reach a backend as SEPARATE math boxes and each box is
            // written on its own. `<mover accent="true"><mrow></mrow>…` renders
            // the mark beside its character instead of over it: visibly
            // approximate, which is the honest failure, and it is exactly the
            // trade `crate::latex::accent_command` documents for `x\hat{}`.
            // The run is marked [`Approx::Approx`] so the mismatch is findable.
            Atom::Glyph { g, .. } if is_all_combining(&g.text) => {
                match pop_element(&mut out) {
                    Some(base) => out.push_str(&accent(&base, &g.text)),
                    None => {
                        *detached = true;
                        out.push_str(&accent("<mrow></mrow>", &g.text));
                    }
                }
                i += 1;
            }
            Atom::Glyph { script: None, .. } => {
                // Adjacent digit records are one number: SATySFi lays out
                // `${3.14}` as four separate glyph records, and four `<mn>`s
                // with an `<mo>.</mo>` between them renders as `3 . 14`,
                // because the dictionary gives an unlisted operator spacing on
                // both sides.
                let (mut base, next) = number_or_glyph(atoms, i);
                i = next;
                // An accent drawn over this base, BEFORE its scripts are
                // collected. The order matters and the case is not exotic:
                // the mark is centred on the base and set at the base's own
                // size or a little smaller, so `mathrec` may classify it as a
                // superscript — and it must still land on the base rather than
                // become one, or `x̂²` would attach the `²` to nothing.
                while let Some(Atom::Glyph { g, .. }) = atoms.get(i) {
                    if !is_all_combining(&g.text) {
                        break;
                    }
                    base = Base {
                        markup: accent(&base.markup, &g.text),
                        operator: false,
                    };
                    i += 1;
                }
                let (subs, sups, limit, next) = take_scripts(atoms, i);
                i = next;
                push_element(&mut out, &attach_scripts(&base, subs, sups, limit));
            }
            // A script with no base before it — a run that opens with one, or
            // what is left after a fraction. MathML's script elements need
            // exactly two or three children, so the base is an empty `<mrow>`
            // rather than nothing.
            Atom::Glyph {
                script: Some(_), ..
            } => {
                let (subs, sups, limit, next) = take_scripts(atoms, i);
                i = next;
                out.push_str(&attach_scripts(
                    &Base {
                        markup: "<mrow></mrow>".to_string(),
                        operator: false,
                    },
                    subs,
                    sups,
                    limit,
                ));
            }
            Atom::Frac { above, below } => {
                let base = Base {
                    markup: format!(
                        "<mfrac>{}{}</mfrac>",
                        row_of(write_atoms(above, detached)),
                        row_of(write_atoms(below, detached)),
                    ),
                    operator: false,
                };
                i += 1;
                let (subs, sups, limit, next) = take_scripts(atoms, i);
                i = next;
                out.push_str(&attach_scripts(&base, subs, sups, limit));
            }
        }
    }
    out
}

/// Append `elem`, folding it into the `<mtext>` immediately before it when
/// both are bare `<mtext>` groups.
///
/// **Measured on `latexcmds`.** A Japanese annotation inside an equation
/// arrives one glyph record per character — `text-in-math` folds a Latin run
/// into one record, but the box stream splits CJK long before that — so
/// `\underset{\text!{運動エネルギーを表す}}` came out as ten `<mtext>`
/// elements, 150 bytes of markup around 30 of content. It renders the same
/// either way; this is about the size and legibility of the file, and it is
/// the same fold `crate::latex::push_prose_merged` does for `\text{…}`.
///
/// Structural rather than a `"</mtext><mtext>"` string replacement, which would
/// also splice across an element boundary that merely ENDS in an `<mtext>` — a
/// `<msub><mi>x</mi><mtext>a</mtext></msub>` followed by a prose record would
/// have the prose pulled inside the subscript. The test is on the whole
/// element, which is a fact this function has and a replacement does not.
fn push_element(out: &mut String, elem: &str) {
    if mtext_body(out).is_some() {
        if let Some(next) = bare_mtext_body(elem) {
            // Reopen the previous group by dropping its `</mtext>`, append the
            // new body, and close the pair once.
            out.truncate(out.len() - "</mtext>".len());
            let _ = write!(out, "{next}</mtext>");
            return;
        }
    }
    out.push_str(elem);
}

/// `out`'s trailing element, if it is a whole bare `<mtext>` — as its escaped
/// body, since that is what [`push_element`] has to splice.
fn mtext_body(out: &str) -> Option<&str> {
    let inner = out.strip_suffix("</mtext>")?;
    let at = inner.rfind("<mtext>")?;
    let body = &inner[at + "<mtext>".len()..];
    // `<mtext>` bodies are `escape_html`'d, so a `<` inside one means the
    // suffix belongs to some other element that merely ends with an `<mtext>`.
    (!body.contains('<') && !inner[..at].ends_with('<')).then_some(body)
}

/// `elem`'s body, if `elem` is exactly one bare `<mtext>` element.
fn bare_mtext_body(elem: &str) -> Option<&str> {
    let body = elem.strip_prefix("<mtext>")?.strip_suffix("</mtext>")?;
    (!body.contains('<')).then_some(body)
}

/// Remove and return `out`'s trailing top-level element, for an accent to be
/// wrapped around.
///
/// Scans back for the matching open tag by counting tag depth. That is exact
/// here and needs no parser, because every character this module writes into
/// an element is `escape_html`'d — so a `<` in the buffer always starts a tag
/// and never appears in content.
fn pop_element(out: &mut String) -> Option<String> {
    if out.is_empty() {
        return None;
    }
    let bytes = out.as_bytes();
    let mut depth = 0i32;
    let mut idx = out.len();
    while idx > 0 {
        let open = out[..idx].rfind('<')?;
        let close = out[open..idx].find('>').map(|n| open + n)?;
        let closing = bytes.get(open + 1) == Some(&b'/');
        let self_closing = bytes[close - 1] == b'/';
        if closing {
            depth += 1;
        } else if !self_closing {
            depth -= 1;
        }
        if depth <= 0 {
            let elem = out[open..].to_string();
            out.truncate(open);
            return Some(elem);
        }
        idx = open;
    }
    None
}

/// One element that scripts may be attached to, and whether it is an `<mo>`.
///
/// The flag is not cosmetic. A `<mo>` carries `movablelimits` from the
/// operator dictionary — true for `∑`, `∏`, `⋃` and the rest of the large
/// operators — and a browser honours it by moving the scripts of an `<msub>`
/// UNDER the operator in display style, and the scripts of a `<munder>` beside
/// it inline. Either way the reader gets a position this port did not measure.
/// [`attach_scripts`] therefore pins it; see there.
struct Base {
    markup: String,
    operator: bool,
}

/// The base at `i`, absorbing following digit records into one `<mn>`.
///
/// Only digits, and only a `.` or `,` with a digit on each side: a decimal
/// point is part of a number, and a comma between two numbers is a separator
/// that belongs in its own `<mo>`. Stops at anything else, including an
/// [`Atom::Space`] — a measured gap between two digits was a gap in the
/// document, not a thousands separator.
fn number_or_glyph<'a>(atoms: &[Atom<'a>], i: usize) -> (Base, usize) {
    let Some(Atom::Glyph { g, script: None }) = atoms.get(i) else {
        unreachable!("called only on an unscripted glyph atom")
    };
    if !all_digits(&g.text) {
        let rec = record_elements(&g.text);
        // A record with several characters is several elements, and only the
        // whole group can take a script — so it becomes one `<mrow>`, which is
        // not an operator whatever its members were.
        let operator = rec.operator && rec.elems.len() == 1;
        return (
            Base {
                markup: row(rec.elems),
                operator,
            },
            i + 1,
        );
    }
    let mut digits = g.text.clone();
    let mut j = i + 1;
    while let Some(Atom::Glyph { g, script: None }) = atoms.get(j) {
        let t = g.text.as_str();
        if all_digits(t) {
            digits.push_str(t);
            j += 1;
            continue;
        }
        // A `.` or `,` continues the number only when a digit follows it.
        if matches!(t, "." | ",")
            && matches!(
                atoms.get(j + 1),
                Some(Atom::Glyph { g: n, script: None }) if all_digits(&n.text)
            )
        {
            digits.push_str(t);
            j += 1;
            continue;
        }
        break;
    }
    (
        Base {
            markup: format!("<mn>{}</mn>", crate::escape_html(&digits)),
            operator: false,
        },
        j,
    )
}

/// A non-empty record made of nothing but ASCII digits.
fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// One glyph record's characters as MathML token elements.
///
/// Shared by the base line and the script positions, and that sharing is the
/// point rather than a convenience: a record classified as PROSE on the base
/// line and as a row of `<mo>`s inside a subscript is the same record read two
/// ways. Measured — `latexcmds`' `\underset{\text!{運動…}}` puts its annotation
/// in a script position, where the per-character path wrote
/// `<mo>運</mo><mo>動</mo>`: CJK characters given the operator dictionary's
/// spacing, in an element whose UA style is `math-auto` italic.
///
/// The `operator` flag is only meaningful for a one-element record; the caller
/// checks the length.
struct Record {
    elems: Vec<String>,
    operator: bool,
}

fn record_elements(text: &str) -> Record {
    // A folded `text-in-math` run, or a CJK character: one `<mtext>` for the
    // whole record, never one per character.
    if is_prose_run(text) {
        return Record {
            elems: vec![format!("<mtext>{}</mtext>", crate::escape_html(text))],
            operator: false,
        };
    }
    let mut elems = Vec::new();
    let mut operator = false;
    let mut digits = String::new();
    let mut letters = String::new();
    // Consecutive UNSTYLED letters are one multi-character `<mi>`, which is
    // MathML's own spelling for a function name (`sin`, `log`) and renders
    // upright with no inter-atom spacing — the same thing a folded
    // `text-in-math` word wants. The layout splits `\text!{if and only if}` at
    // every glue, so its words arrive as separate records holding no space and
    // `is_prose_run` cannot see them; letter-by-letter they would be eight
    // `<mi>`s where two suffice. STYLED letters are excluded: `𝑥𝑦` is two
    // variables multiplied, and `<mi>𝑥𝑦</mi>` would claim it is one name.
    let flush = |elems: &mut Vec<String>, letters: &mut String| {
        if letters.is_empty() {
            return;
        }
        if letters.chars().count() == 1 {
            let c = letters.chars().next().expect("length checked");
            elems.push(identifier(c));
        } else {
            // No `mathvariant`: `math-auto` italicises only a SINGLE
            // character, so a multi-letter `<mi>` is already upright.
            elems.push(format!("<mi>{}</mi>", crate::escape_html(letters)));
        }
        letters.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() {
            flush(&mut elems, &mut letters);
            digits.push(c);
            continue;
        }
        if !digits.is_empty() {
            elems.push(format!("<mn>{digits}</mn>"));
            digits.clear();
        }
        if is_math_letter(c) && rustyfi_backend::math_alphanumeric_base(c).is_none() {
            letters.push(c);
            continue;
        }
        flush(&mut elems, &mut letters);
        if is_math_letter(c) {
            elems.push(identifier(c));
        } else {
            elems.push(format!("<mo>{}</mo>", crate::escape_html(&c.to_string())));
            operator = true;
        }
    }
    if !digits.is_empty() {
        elems.push(format!("<mn>{digits}</mn>"));
    }
    flush(&mut elems, &mut letters);
    Record { elems, operator }
}

/// A letter as `<mi>`, with `mathvariant="normal"` where Core would otherwise
/// italicise it.
///
/// **The attribute is the load-bearing part.** MathML Core's UA stylesheet
/// applies `text-transform: math-auto` to an `<mi>` holding a single character,
/// which maps a basic Latin or Greek letter to its ITALIC Mathematical
/// Alphanumeric counterpart. For hand-written MathML that is a convenience.
/// Here it is a bug: SATySFi's math writes the style into the codepoint —
/// `MathCharClass::Italic` is the default and `default_math_variant_char`
/// (`rustyfi_backend::math`, upstream `primitives.cppo.ml:358-460`) maps `x` to
/// `𝑥` U+1D465 before layout — so a plain ASCII `x` reaching a backend is a
/// letter the document set UPRIGHT, through `\mathrm`-style
/// `MathCharClass::Roman` or through `text-in-math`. Letting the browser
/// italicise it renders the opposite of what the PDF shows.
///
/// A letter that IS a Mathematical Alphanumeric Symbol needs nothing: it is
/// outside `math-auto`'s domain, so it passes through as itself and keeps the
/// bold, script, fraktur or double-struck style the codepoint carries. That is
/// the same fact `crate::latex::style_wrapper` exploits, spent here on an
/// attribute that is not written rather than on a `\mathbb{…}` that is.
fn identifier(c: char) -> String {
    let styled = rustyfi_backend::math_alphanumeric_base(c).is_some();
    let body = crate::escape_html(&c.to_string());
    if styled {
        format!("<mi>{body}</mi>")
    } else {
        format!("<mi mathvariant=\"normal\">{body}</mi>")
    }
}

/// Wrap `base` in `<mover accent="true">` with `marks` over it.
///
/// `accent="true"` is what tells the renderer to tuck the mark down onto the
/// base rather than stack it at superscript height — the difference between
/// `x̂` and `x^`. The marks are emitted as `<mo>` because Core's UA stylesheet
/// gives `<mo>` no `math-auto`, so a combining character is not silently
/// transformed on its way through.
fn accent(base: &str, marks: &str) -> String {
    format!(
        "<mover accent=\"true\">{base}<mo>{}</mo></mover>",
        crate::escape_html(marks),
    )
}

/// Attach the scripts collected by [`take_scripts`] to `base`.
///
/// **`movablelimits="false"` on an operator base, in every case.** The operator
/// dictionary marks `∑`, `∏`, `⋃`, `⋀` and the rest `movablelimits`, which
/// means a browser moves their scripts to wherever the current math-style says
/// they belong — under and over in display, beside inline. This port already
/// MEASURED where they belong: [`crate::mathrec`]'s `Script::limit` is exact
/// rather than a threshold (`layout_math_list` centres a limit on the base's
/// advance and sets a script AT it), so letting the dictionary decide would
/// overwrite a measurement with a convention, and would do it differently
/// depending on whether the equation happened to be displayed.
///
/// It matters in both directions, which is why the attribute is unconditional
/// on an `<mo>` base rather than only on the `<munderover>` arms: an `∫` whose
/// limits the layout set BESIDE it — `layout_math_list` does, at the display
/// variant's advance, see `crate::latex`'s
/// `a_gap_before_a_script_does_not_detach_it` — becomes an `<msubsup>`, and a
/// `∑` in the same shape would be re-stacked by the browser the moment the
/// paragraph turned out to be a display block.
fn attach_scripts(base: &Base, subs: Vec<String>, sups: Vec<String>, limit: bool) -> String {
    if subs.is_empty() && sups.is_empty() {
        return base.markup.clone();
    }
    let markup = if base.operator {
        pin_limits(&base.markup)
    } else {
        base.markup.clone()
    };
    match (subs.is_empty(), sups.is_empty()) {
        (true, true) => unreachable!("handled above"),
        (false, true) => {
            let tag = if limit { "munder" } else { "msub" };
            format!("<{tag}>{markup}{}</{tag}>", row(subs))
        }
        (true, false) => {
            let tag = if limit { "mover" } else { "msup" };
            format!("<{tag}>{markup}{}</{tag}>", row(sups))
        }
        (false, false) => {
            let tag = if limit { "munderover" } else { "msubsup" };
            format!("<{tag}>{markup}{}{}</{tag}>", row(subs), row(sups))
        }
    }
}

/// Add `movablelimits="false"` to an `<mo>` open tag. A string edit rather than
/// a flag threaded through [`element_for`], because the base is only known to
/// be a script base one call later and every other use of an `<mo>` must not
/// carry the attribute — 24 bytes on every operator in the document, for a
/// property only a script base has.
fn pin_limits(mo: &str) -> String {
    match mo.strip_prefix("<mo>") {
        Some(rest) => format!("<mo movablelimits=\"false\">{rest}"),
        None => mo.to_string(),
    }
}

/// One script position's parts as a single child: the part itself when there is
/// one, an `<mrow>` when there are several, and an empty `<mrow>` when there
/// are none.
///
/// MathML's script and fraction elements are strict about their child COUNT —
/// `<msubsup>` takes exactly three — so a two-glyph subscript that is written
/// as two children silently becomes a different element's worth of arguments.
fn row(mut parts: Vec<String>) -> String {
    match parts.len() {
        0 => "<mrow></mrow>".to_string(),
        1 => parts.pop().expect("length checked"),
        _ => format!("<mrow>{}</mrow>", parts.concat()),
    }
}

/// [`row`] for a body that has already been serialized — a fraction's half.
///
/// `<mfrac>` takes exactly two children, so a half holding two elements MUST
/// be wrapped or the fraction silently becomes a three-child element whose
/// third child a browser ignores. A half holding exactly one is not wrapped,
/// which is what keeps `1/2` from costing 26 bytes of `<mrow>` for nothing;
/// "exactly one" is decided by [`pop_element`], the same scan the accent
/// handling uses, rather than by counting `<`.
fn row_of(body: String) -> String {
    let mut probe = body.clone();
    if pop_element(&mut probe).is_some() && probe.is_empty() {
        return body;
    }
    format!("<mrow>{body}</mrow>")
}

/// Consume the maximal run of script atoms starting at `i`, returning the
/// subscript's parts, the superscript's parts, whether they were set as LIMITS,
/// and the index after them.
///
/// Both directions in ONE pass, for the reason `crate::latex::take_scripts`
/// documents: `layout_math_list` centres an upper and a lower limit on the same
/// base, so `\sum_{k=1}^{n}`'s `n` sorts between the `k` and the `1` in `dx`
/// order and the two cannot be separated by position.
///
/// A space INSIDE a script run is kept (a subscript can read `k = 1`); a space
/// that merely PRECEDES the first script is swallowed, because
/// `layout_math_list` sets an integral's scripts at the display variant's
/// advance and leaves a measured gap there that is not a space in the document.
fn take_scripts(atoms: &[Atom<'_>], mut i: usize) -> (Vec<String>, Vec<String>, bool, usize) {
    let mut subs: Vec<String> = Vec::new();
    let mut sups: Vec<String> = Vec::new();
    let mut limit = false;
    let mut seen = false;
    while i < atoms.len() {
        match &atoms[i] {
            // An accent inside a script run belongs to the script glyph before
            // it, exactly as it does on the base line.
            //
            // **Only when there IS one.** An accent on the base is classified
            // as a superscript by the same size-and-shift test that finds a
            // real one — it is centred over the base and set a little smaller —
            // so it reaches here first, with the side empty. Consuming it then
            // would drop it AND leave the base bare, which is what `\hat{x}`
            // did until the browser comparison showed the hat missing.
            // Declining lets `write_atoms`'s own accent loop have it.
            Atom::Glyph {
                g,
                script: Some(Script { up, .. }),
            } if is_all_combining(&g.text) => {
                let side = if *up { &mut sups } else { &mut subs };
                let Some(base) = side.pop() else { break };
                side.push(accent(&base, &g.text));
                i += 1;
            }
            Atom::Glyph {
                g,
                script: Some(Script { up, limit: lim }),
            } => {
                seen = true;
                limit |= *lim;
                let side = if *up { &mut sups } else { &mut subs };
                // Flat rather than one `<mrow>` per record: a script position
                // takes exactly one child, and [`row`] wraps the whole side
                // once at the end. A `<mrow>` per record would nest a group
                // inside a group for every glyph of a multi-token limit.
                side.extend(record_elements(&g.text).elems);
                i += 1;
            }
            Atom::Space
                if matches!(
                    atoms.get(i + 1),
                    Some(Atom::Glyph {
                        script: Some(_),
                        ..
                    })
                ) =>
            {
                // Which SIDE a gap between two scripts belongs to is not
                // recoverable, so it goes to whichever is open, preferring the
                // subscript — where a multi-token limit almost always is.
                if seen {
                    if subs.is_empty() && !sups.is_empty() {
                        sups.push("<mspace width=\"0.25em\"/>".to_string());
                    } else if !subs.is_empty() {
                        subs.push("<mspace width=\"0.25em\"/>".to_string());
                    }
                }
                i += 1;
            }
            _ => break,
        }
    }
    (subs, sups, limit, i)
}

/// Would the operator dictionary put this space in by itself?
///
/// **The same question `crate::latex::latex_spaces_itself` asks, and the same
/// answer, because MathML Core's operator dictionary IS TeX's spacing classes**
/// — Core's Appendix A is generated from the Unicode math property tables that
/// the `Bin`/`Rel`/`Op` classes are, and a browser applies each entry's
/// `lspace`/`rspace` around the `<mo>` with no help from this module. A gap
/// recovered either side of a `+` is therefore DERIVED information: SATySFi's
/// math layout inserted it by those very rules, and re-emitting it as an
/// `<mspace>` renders the equation wider than the PDF.
///
/// What is left over, and is kept, is the spacing the dictionary does NOT
/// supply: the gap between two ordinary atoms, which is the only surviving
/// trace of the word spaces inside a `text-in-math` run.
///
/// The classification is `crate::latex::auto_spaced`'s, reused rather than
/// re-tabulated — one predicate, two writers, for the reason
/// [`crate::mathrec`] exists at all.
fn dictionary_spaces_it(atoms: &[Atom<'_>], i: usize) -> bool {
    let side = |idx: Option<usize>| -> bool {
        match idx.and_then(|j| atoms.get(j)) {
            Some(Atom::Glyph { g, .. }) => g.text.chars().any(crate::latex::auto_spaced),
            _ => false,
        }
    };
    side(i.checked_sub(1)) || side(Some(i + 1))
}

/// Is this record ordinary PROSE that happened to be set inside an equation?
///
/// The same two structural signals `crate::latex::is_prose_run` uses — it holds
/// a space, or it holds a character with no reading as mathematics — but the
/// second test is by SCRIPT rather than by symbol table. `--katex` can ask "is
/// there a LaTeX command for this character", and MathML has no such list to
/// consult: a symbol with no name is still `<mo>` and renders as itself. So
/// what is asked instead is whether the character is a letter of a writing
/// system rather than a mathematical letter, which catches exactly the
/// population that reaches an equation through `text-in-math`: CJK, kana,
/// Hangul, accented Latin.
fn is_prose_run(text: &str) -> bool {
    text.contains(' ') || text.chars().any(|c| c.is_alphabetic() && !is_math_letter(c))
}

/// A letter mathematics writes directly: ASCII, Greek and Coptic, and anything
/// Unicode encodes as a styled mathematical letter — the Mathematical
/// Alphanumeric Symbols block plus the Letterlike Symbols that fill its holes
/// (`ℝ`, `ℋ`, `ℭ`), which `math_alphanumeric_base` already enumerates.
fn is_math_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
        || matches!(c as u32, 0x370..=0x3FF)
        || rustyfi_backend::math_alphanumeric_base(c).is_some()
}

/// Does this record hold nothing but combining marks — i.e. is it an accent the
/// math layout placed over the character before it?
///
/// The ranges are `crate::latex::is_combining`'s, and the emptiness guard
/// matters: an advance-only record has no characters at all, and `all` on an
/// empty iterator is true, which would make it an accent and eat the element
/// before it.
fn is_all_combining(text: &str) -> bool {
    !text.is_empty() && text.chars().all(crate::latex::is_combining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathrec::tests::glyph;
    use rustyfi_backend::{Closing, Color, Length, Path, PathSeg, Subpath};

    fn ml(glyphs: &[MathGlyph]) -> String {
        math_mathml(glyphs, &[]).0
    }

    /// A wide flat fill, the shape a fraction bar has.
    fn bar(x0: f64, x1: f64, y: f64) -> GraphicsElem {
        GraphicsElem::Fill(
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: (Length(x0), Length(y)),
                    segs: vec![
                        PathSeg::Line((Length(x1), Length(y))),
                        PathSeg::Line((Length(x1), Length(y + 0.5))),
                        PathSeg::Line((Length(x0), Length(y + 0.5))),
                    ],
                    closing: Closing::Line,
                }],
            },
        )
    }

    /// A tall filled shape — a `math-paren` delimiter, which is what makes a
    /// run [`Approx::Approx`].
    fn blob() -> GraphicsElem {
        GraphicsElem::Fill(
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: (Length(0.0), Length(0.0)),
                    segs: vec![
                        PathSeg::Line((Length(2.0), Length(0.0))),
                        PathSeg::Line((Length(2.0), Length(20.0))),
                        PathSeg::Line((Length(0.0), Length(20.0))),
                    ],
                    closing: Closing::Line,
                }],
            },
        )
    }

    /// The elements MathML Core defines. Anything outside this list may render
    /// today and stop rendering tomorrow, or render in one engine and not
    /// another — which is the whole reason this mode targets Core rather than
    /// MathML 3.
    const CORE: [&str; 22] = [
        "math", "mrow", "mi", "mn", "mo", "ms", "mtext", "mspace", "mfrac", "msqrt", "mroot",
        "mstyle", "merror", "mpadded", "mphantom", "msub", "msup", "msubsup", "munder", "mover",
        "munderover", "mmultiscripts",
    ];

    /// Every element name this module can write is in MathML Core.
    ///
    /// Asserted over a fixture exercising each construct rather than by reading
    /// the source, so a future writer that reaches for `<mfenced>` — MathML 3's
    /// delimiter element, which no browser implements — fails here rather than
    /// in someone's browser.
    #[test]
    fn every_element_is_mathml_core() {
        let mut markup = String::new();
        markup.push_str(&ml(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("2", 5.0, 4.0, 7.0),
            glyph("\u{2211}", 12.0, 0.0, 12.0),
            glyph("k", 12.5, -6.0, 8.0),
            glyph("n", 13.5, 7.0, 8.0),
            glyph("if and only if", 30.0, 0.0, 10.0),
            glyph("\u{302}", 45.0, 0.0, 10.0),
        ]));
        markup.push_str(
            &math_mathml(
                &[glyph("a", 2.0, 8.0, 10.0), glyph("c", 8.0, -4.0, 10.0)],
                &[bar(0.0, 20.0, 4.0)],
            )
            .0,
        );
        markup.push_str(&open_tag(true, Approx::Exact));
        markup.push_str(CLOSE_TAG);
        for tag in markup.split('<').skip(1) {
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            assert!(
                CORE.contains(&name.as_str()),
                "`{name}` is not a MathML Core element:\n{markup}"
            );
        }
    }

    /// A script beside its base is `<msub>`/`<msup>`; a script CENTRED on it is
    /// `<munder>`/`<mover>`. Unlike `--katex`, which may only write `\limits`
    /// after one of fifteen commands, the geometry is emitted as measured.
    #[test]
    fn a_centred_script_becomes_a_limit_and_a_trailing_one_a_script() {
        let limits = ml(&[
            glyph("\u{2211}", 0.0, 0.0, 12.0),
            glyph("k", 0.5, -6.0, 8.0),
            glyph("n", 1.5, 7.0, 8.0),
        ]);
        assert!(limits.contains("<munderover>"), "{limits}");
        // …and the base is pinned, or the browser re-decides where the limits
        // go the moment the equation is displayed.
        assert!(limits.contains("movablelimits=\"false\""), "{limits}");

        let beside = ml(&[glyph("x", 0.0, 0.0, 10.0), glyph("2", 5.0, 4.0, 7.0)]);
        assert!(beside.contains("<msup>"), "{beside}");
        assert!(!beside.contains("munder"), "{beside}");

        // A centred script on an ORDINARY letter is a limit here, where
        // `--katex` has to demote it: `\limits` after a non-operator is a hard
        // KaTeX error, `<munder>` on an `<mi>` is not.
        let on_a_letter = ml(&[glyph("R", 0.0, 0.0, 12.0), glyph("k", 0.5, -6.0, 8.0)]);
        assert!(on_a_letter.contains("<munder>"), "{on_a_letter}");
    }

    /// A multi-glyph limit is ONE script, not a staircase of nested ones.
    ///
    /// The failure this prevents is the same one `--katex`'s
    /// `a_multi_glyph_limit_becomes_one_subscript_group` prevents, in a
    /// vocabulary that does not reject it: LaTeX refuses a double subscript
    /// outright, whereas `<msub><msub><msub>` renders happily as three
    /// descending steps and nothing anywhere says it is wrong.
    #[test]
    fn a_multi_glyph_limit_is_one_script_element() {
        let out = ml(&[
            glyph("\u{2211}", 0.0, 0.0, 12.0),
            glyph("k", 0.5, -6.0, 8.0),
            glyph("n", 1.5, 7.0, 8.0),
            glyph("=", 2.0, -6.0, 8.0),
            glyph("1", 3.5, -6.0, 8.0),
        ]);
        assert_eq!(out.matches("<munderover>").count(), 1, "{out}");
        assert_eq!(out.matches("<munder>").count(), 0, "{out}");
        // The subscript's three glyphs are one `<mrow>` child.
        assert!(out.contains("<mrow><mi"), "{out}");
    }

    /// An unstyled Latin letter is one the document set UPRIGHT, because the
    /// italic is written into the codepoint before layout. Without
    /// `mathvariant="normal"` the browser italicises it and renders the
    /// opposite of the PDF.
    #[test]
    fn an_unstyled_letter_is_pinned_upright_and_a_styled_one_is_not() {
        assert_eq!(
            ml(&[glyph("x", 0.0, 0.0, 10.0)]),
            "<mi mathvariant=\"normal\">x</mi>"
        );
        // Math italic: outside `math-auto`'s domain, so it needs no attribute
        // and keeps its own style.
        assert_eq!(ml(&[glyph("\u{1D465}", 0.0, 0.0, 10.0)]), "<mi>\u{1D465}</mi>");
        // …and so does every other alphabet the codepoint can carry.
        assert_eq!(ml(&[glyph("\u{1D538}", 0.0, 0.0, 10.0)]), "<mi>\u{1D538}</mi>");
        assert_eq!(ml(&[glyph("\u{211D}", 0.0, 0.0, 10.0)]), "<mi>\u{211D}</mi>");
    }

    /// A fraction bar is the one structure the box stream still carries.
    #[test]
    fn a_fraction_bar_becomes_an_mfrac_and_is_not_marked_approximate() {
        let (out, approx) = math_mathml(
            &[
                glyph("a", 2.0, 8.0, 10.0),
                glyph("+", 7.0, 8.0, 10.0),
                glyph("b", 12.0, 8.0, 10.0),
                glyph("c", 8.0, -4.0, 10.0),
            ],
            &[bar(0.0, 20.0, 4.0)],
        );
        assert!(out.starts_with("<mfrac><mrow>"), "{out}");
        assert!(out.contains("<mo>+</mo>"), "{out}");
        assert_eq!(out.matches("<mfrac>").count(), 1, "{out}");
        // The bar is the only ink and it was used, so nothing is unaccounted
        // for.
        assert_eq!(approx, Approx::Exact);
    }

    /// Ink that did not become a fraction bar — a `math-paren` delimiter, a
    /// radical sign — is drawn in the PDF and has no MathML spelling. The
    /// equation is still written, and it says so.
    #[test]
    fn unaccounted_ink_marks_the_run_approximate() {
        let (out, approx) = math_mathml(&[glyph("x", 0.0, 0.0, 10.0)], &[blob()]);
        assert_eq!(approx, Approx::Approx);
        // Nothing is invented to stand in for it: no delimiter, no radical.
        assert!(!out.contains("msqrt"), "{out}");
        assert!(!out.contains("mo>(") && !out.contains("mo>)"), "{out}");
        // The class the caller writes, and the merge rule for a displayed
        // equation built out of several boxes.
        assert!(open_tag(false, approx).contains(APPROX_CLASS));
        assert!(!open_tag(false, Approx::Exact).contains(APPROX_CLASS));
        assert_eq!(Approx::Exact.or(Approx::Approx), Approx::Approx);
        assert_eq!(Approx::Exact.or(Approx::Exact), Approx::Exact);
    }

    /// An accent attaches to the character it was placed over — the one thing
    /// this mode recovers that `--katex` structurally cannot.
    #[test]
    fn an_accent_attaches_to_its_base() {
        let out = ml(&[
            glyph("x", 0.0, 0.0, 10.0),
            // A combining circumflex, positioned over the `x`.
            glyph("\u{302}", 1.0, 0.0, 10.0),
        ]);
        assert_eq!(
            out,
            "<mover accent=\"true\"><mi mathvariant=\"normal\">x</mi><mo>\u{302}</mo></mover>"
        );
        // With nothing to accent — `azmath` builds every accent out of two
        // `draw-text`s, so the mark reaches a backend as its own math box —
        // the mark goes over an EMPTY group rather than being dropped, and the
        // run says it is approximate.
        let (orphan, approx) = math_mathml(&[glyph("\u{302}", 0.0, 0.0, 10.0)], &[]);
        assert_eq!(
            orphan,
            "<mover accent=\"true\"><mrow></mrow><mo>\u{302}</mo></mover>"
        );
        assert_eq!(approx, Approx::Approx);
    }

    /// **The bug the browser comparison found.** A mark drawn over its base is
    /// centred on the base and set a little smaller, which is exactly the test
    /// `mathrec` uses for a SUPERSCRIPT — so an accent reaches the script
    /// collector before anything else does, with no script yet to attach to.
    /// Consuming it there dropped it silently: `azmath`'s `${\hat{x}}` rendered
    /// as a bare `x` in a real browser while every unit test passed.
    ///
    /// It must also come off the base BEFORE the base's own scripts, or `x̂²`
    /// hangs the `²` off an empty group.
    #[test]
    fn an_accent_classified_as_a_script_still_lands_on_its_base() {
        // Smaller and raised: `classify_script` calls this a superscript.
        let out = ml(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("\u{302}", 1.0, 5.0, 7.0),
        ]);
        assert_eq!(
            out,
            "<mover accent=\"true\"><mi mathvariant=\"normal\">x</mi><mo>\u{302}</mo></mover>",
            "the accent was eaten by the script collector"
        );
        // …and a REAL script after it still finds the accented base.
        let scripted = ml(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("\u{302}", 1.0, 5.0, 7.0),
            glyph("2", 5.0, 5.0, 7.0),
        ]);
        assert_eq!(
            scripted,
            "<msup><mover accent=\"true\"><mi mathvariant=\"normal\">x</mi>\
             <mo>\u{302}</mo></mover><mn>2</mn></msup>"
        );
        // A mark inside a genuine multi-glyph script still binds there.
        let in_script = ml(&[
            glyph("f", 0.0, 0.0, 10.0),
            glyph("a", 5.0, 5.0, 7.0),
            glyph("\u{302}", 5.5, 5.0, 7.0),
        ]);
        assert!(
            in_script.starts_with("<msup>") && in_script.contains("<mover accent=\"true\">"),
            "{in_script}"
        );
    }

    /// The operator dictionary spaces `+` itself, so a recovered gap around one
    /// must not be written again; a gap between two ordinary atoms is the only
    /// trace of a word space in a `text-in-math` run and must be.
    #[test]
    fn the_dictionary_supplies_operator_spacing_but_not_a_word_space() {
        let spaced = ml(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("+", 12.0, 0.0, 10.0),
            glyph("1", 20.0, 0.0, 10.0),
        ]);
        assert!(!spaced.contains("mspace"), "spacing doubled:\n{spaced}");

        let words = ml(&[glyph("if", 0.0, 0.0, 10.0), glyph("and", 12.0, 0.0, 10.0)]);
        assert!(words.contains("<mspace width=\"0.25em\"/>"), "{words}");
    }

    /// Prose set inside an equation is `<mtext>`, not a row of italic
    /// identifiers. Both signals: a folded `text-in-math` run holds a space,
    /// and a CJK annotation arrives one character per record and holds none.
    #[test]
    fn prose_inside_an_equation_becomes_mtext() {
        assert_eq!(
            ml(&[glyph("if and only if", 0.0, 0.0, 10.0)]),
            "<mtext>if and only if</mtext>"
        );
        assert_eq!(
            ml(&[glyph("\u{904B}", 0.0, 0.0, 10.0)]),
            "<mtext>\u{904B}</mtext>"
        );
        // A multi-character record that is NOT prose stays mathematics.
        assert_eq!(ml(&[glyph("12", 0.0, 0.0, 10.0)]), "<mn>12</mn>");
    }

    /// A record of several unstyled letters is ONE `<mi>` — MathML's spelling
    /// for a function name, and what a folded `text-in-math` word wants too.
    ///
    /// The layout splits `\text!{if and only if}` at every glue, so its words
    /// arrive as separate records holding no space: [`is_prose_run`] cannot
    /// see them, and letter-by-letter they come out as eight `<mi>`s where two
    /// suffice. Measured on `math-selection.saty`.
    #[test]
    fn a_run_of_unstyled_letters_is_one_identifier() {
        assert_eq!(ml(&[glyph("if", 0.0, 0.0, 10.0)]), "<mi>if</mi>");
        assert_eq!(ml(&[glyph("sin", 0.0, 0.0, 10.0)]), "<mi>sin</mi>");
        // A single letter still takes the upright pin; a multi-letter `<mi>`
        // is already upright, so it must NOT (the attribute would be noise).
        assert_eq!(
            ml(&[glyph("x", 0.0, 0.0, 10.0)]),
            "<mi mathvariant=\"normal\">x</mi>"
        );
        // Styled letters are two variables multiplied, not one name.
        assert_eq!(
            ml(&[glyph("\u{1D465}\u{1D466}", 0.0, 0.0, 10.0)]),
            "<mrow><mi>\u{1D465}</mi><mi>\u{1D466}</mi></mrow>"
        );
        // Letters and digits in one record still separate.
        assert_eq!(
            ml(&[glyph("x1", 0.0, 0.0, 10.0)]),
            "<mrow><mi mathvariant=\"normal\">x</mi><mn>1</mn></mrow>"
        );
    }

    /// Markup-hostile characters are escaped, or the equation ends the element
    /// it is in. `<` is the one that would otherwise open a tag.
    #[test]
    fn markup_characters_are_escaped() {
        let out = ml(&[
            glyph("<", 0.0, 0.0, 10.0),
            glyph("&", 5.0, 0.0, 10.0),
            glyph(">", 10.0, 0.0, 10.0),
        ]);
        assert_eq!(out, "<mo>&lt;</mo><mo>&amp;</mo><mo>&gt;</mo>");
    }

    /// Adjacent digit records are one number. SATySFi lays out `${3.14}` as
    /// separate glyph records, and an `<mo>.</mo>` between two `<mn>`s takes
    /// the dictionary's spacing on both sides — `3 . 14`.
    #[test]
    fn adjacent_digits_are_one_number() {
        let out = ml(&[
            glyph("3", 0.0, 0.0, 10.0),
            glyph(".", 5.0, 0.0, 10.0),
            glyph("1", 8.0, 0.0, 10.0),
            glyph("4", 13.0, 0.0, 10.0),
        ]);
        assert_eq!(out, "<mn>3.14</mn>");
        // A trailing separator is punctuation, not part of the number.
        let out = ml(&[glyph("1", 0.0, 0.0, 10.0), glyph(",", 5.0, 0.0, 10.0)]);
        assert_eq!(out, "<mn>1</mn><mo>,</mo>");
    }

    /// Consecutive prose records fold into ONE `<mtext>`. CJK reaches an
    /// equation one character per record, so `latexcmds`'
    /// `\underset{\text!{運動エネルギーを表す}}` is ten records — and without
    /// the fold, 150 bytes of markup around 30 of content.
    #[test]
    fn adjacent_prose_records_merge_into_one_mtext() {
        let out = ml(&[
            glyph("\u{904B}", 0.0, 0.0, 10.0),
            glyph("\u{52D5}", 5.0, 0.0, 10.0),
            glyph("\u{91CF}", 10.0, 0.0, 10.0),
        ]);
        assert_eq!(out, "<mtext>\u{904B}\u{52D5}\u{91CF}</mtext>");

        // …but only across two BARE groups. An element that merely ENDS in an
        // `<mtext>` — a subscript whose script is prose — must not have the
        // next record pulled inside it, which is a different equation. A
        // `"</mtext><mtext>"` string replacement would do exactly that; this
        // shape is why the test is on the whole element.
        let after_script = ml(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("\u{904B}", 5.0, -4.0, 7.0),
            glyph("\u{52D5}", 8.5, 0.0, 10.0),
        ]);
        assert_eq!(
            after_script,
            "<msub><mi mathvariant=\"normal\">x</mi><mtext>\u{904B}</mtext></msub>\
             <mtext>\u{52D5}</mtext>",
        );
    }

    /// [`pop_element`] takes the whole trailing element, however deeply nested
    /// — which is what makes an accent land on the construct it was drawn over
    /// rather than on its last token.
    #[test]
    fn popping_an_element_respects_nesting() {
        let mut s = String::from("<mi>a</mi><msub><mi>x</mi><mn>2</mn></msub>");
        assert_eq!(
            pop_element(&mut s).as_deref(),
            Some("<msub><mi>x</mi><mn>2</mn></msub>")
        );
        assert_eq!(s, "<mi>a</mi>");
        // A self-closing element is one element, not an unbalanced open tag.
        let mut s = String::from("<mi>a</mi><mspace width=\"0.25em\"/>");
        assert_eq!(pop_element(&mut s).as_deref(), Some("<mspace width=\"0.25em\"/>"));
        assert_eq!(s, "<mi>a</mi>");
        assert_eq!(pop_element(&mut String::new()), None);
    }

    /// `<mfrac>` takes exactly two children, so a multi-element half must be
    /// wrapped — and a single-element half must not be, or every `1/2` in the
    /// document carries 26 bytes of `<mrow>` for nothing.
    #[test]
    fn a_fraction_half_is_wrapped_only_when_it_needs_to_be() {
        assert_eq!(row_of("<mn>1</mn>".to_string()), "<mn>1</mn>");
        assert_eq!(
            row_of("<mn>1</mn><mo>+</mo>".to_string()),
            "<mrow><mn>1</mn><mo>+</mo></mrow>"
        );
        assert_eq!(row_of(String::new()), "<mrow></mrow>");
        // A nested element is still ONE child.
        assert_eq!(
            row_of("<msub><mi>x</mi><mn>2</mn></msub>".to_string()),
            "<msub><mi>x</mi><mn>2</mn></msub>"
        );
    }

    /// **The body never contains a newline**, in any construct.
    ///
    /// This is the writer's half of the CommonMark constraint the module
    /// comment states. A renderer with `breaks: true` inserts a `<br>` at every
    /// newline inside inline HTML, and a BLANK line terminates the HTML block
    /// outright and leaves the rest of the element as literal text.
    ///
    /// Asserted HERE rather than on a rendered `.md`, and the difference
    /// matters: `Para::render` runs `collapse_spaces` over every non-code
    /// paragraph, which maps `\n` to a space — so a file-level version of this
    /// test cannot fail however this module is broken, and would be a green
    /// light with nothing behind it.
    #[test]
    fn the_body_is_always_one_line() {
        let (body, _) = math_mathml(
            &[
                glyph("\u{2211}", 0.0, 0.0, 12.0),
                glyph("k", 0.5, -6.0, 8.0),
                glyph("n", 1.5, 7.0, 8.0),
                glyph("a", 12.0, 8.0, 10.0),
                glyph("c", 14.0, -4.0, 10.0),
                glyph("if and only if", 30.0, 0.0, 10.0),
                glyph("\u{302}", 60.0, 0.0, 10.0),
            ],
            &[bar(10.0, 20.0, 4.0)],
        );
        assert!(!body.contains('\n'), "a newline in the body:\n{body}");
        assert!(!body.contains('\r'), "{body}");
    }

    /// An empty run writes nothing, so the caller never emits a bare
    /// `<math></math>` that reserves space for an equation that is not there.
    #[test]
    fn an_empty_run_produces_no_children() {
        assert!(math_mathml(&[], &[]).0.is_empty());
    }
}
