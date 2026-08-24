//! `--katex`: a math run written back as LaTeX, for a reader whose renderer
//! runs KaTeX or MathJax.
//!
//! ## What this can and cannot be
//!
//! It is a RE-DERIVATION, not a round trip. The document's `${…}` source is
//! gone by the time any backend runs — see [`crate::mathrec`]'s doc comment
//! for where — so this writes LaTeX for the structure that can be read back
//! out of the laid-out glyphs, and nothing else. Everything below is a
//! consequence of that one fact, and each item is a thing this deliberately
//! does NOT emit rather than a thing it emits wrongly:
//!
//! | construct | what happens | why |
//! |--|--|--|
//! | `\sqrt{x}` | the radicand, unwrapped | the radical SIGN is a `rules` path, not a glyph; there is no `√` in the run to key on and no way to tell the radicand from the text beside it |
//! | matrices, `\begin{aligned}` | the cells, in `dx` order | row/column arrangement is carried by position alone and no bar delimits it |
//! | a fraction inside a fraction | flattened into the outer one | [`crate::mathrec`]'s "one level deep" |
//! | `\text{…}` | its characters, in math mode | `math_boxes_of_inline_boxes` folds the run into one glyph record and keeps no mark that it was upright text, so re-wrapping it in `\text` would be a guess |
//! | `\left(…\right)` | the delimiter characters | a grown delimiter arrives as one `MathGlyph` per assembly PART; only its size is lost, and the character is right |
//! | bold/script/fraktur | recovered (see [`style_wrapper`]) | Unicode's Mathematical Alphanumeric Symbols encode it, so this one IS recoverable |
//! | colour, explicit spacing | dropped / approximated | a `\,` and a `\;` are both "a gap wider than the threshold" by the time this runs |
//!
//! **Nothing is emitted that would render differently from the PDF without
//! saying so.** Where the structure is genuinely unavailable the characters go
//! out as themselves, which renders as the same characters — legible, and
//! visibly not a fraction — rather than as a construct this module guessed at.
//! The one place that judgement is exercised is [`Escape`]: a character with
//! no LaTeX spelling is escaped so it renders literally, never dropped.
//!
//! ## Delimiters are the CALLER's, not this module's
//!
//! [`math_latex`] returns the body only. Markdown wraps it in `$…$`/`$$…$$`
//! (what GitHub, Pandoc, VS Code and Typora read) and the HTML backend in
//! `\(…\)`/`\[…\]` (what KaTeX's `auto-render` and MathJax read BY DEFAULT —
//! neither enables `$…$` for inline math without configuration, so emitting it
//! into an HTML page would render as a literal dollar sign on a reader's
//! default setup). Same body, two conventions, because the two ecosystems
//! genuinely differ.

use rustyfi_backend::MathGlyph;

use crate::mathrec::{self, Atom, Script};

/// One `PureHorzBox::Math` as a LaTeX math-mode BODY — no `$`, no `\(`, see
/// this module's doc comment. Empty when the run holds nothing this layer can
/// name, in which case the caller should write nothing at all rather than an
/// empty `$$`.
pub(crate) fn math_latex(
    glyphs: &[MathGlyph],
    rules: &[rustyfi_backend::GraphicsElem],
) -> String {
    write_atoms(&mathrec::recover(glyphs, rules))
}

/// Fold a recovered atom list into LaTeX.
///
/// The loop is not one-atom-at-a-time, because a script is not a token: LaTeX
/// permits exactly one `_` and one `^` per base, and [`crate::mathrec`] hands
/// back one atom per GLYPH RECORD — so `\sum_{k=1}^{n}` arrives as a base and
/// four separate script atoms. Emitting them individually produces
/// `\sum_{k}_{=}_{1}^{n}`, which is not merely ugly: LaTeX and KaTeX both
/// REJECT a double subscript, so the whole equation would fail to render. See
/// [`take_scripts`].
fn write_atoms(atoms: &[Atom<'_>]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < atoms.len() {
        match &atoms[i] {
            // A leading space carries no information and `$ x$` is not what
            // the document said. The guard is on the OUTPUT rather than on the
            // index for the reason `mathrec::recover` documents.
            Atom::Space => {
                if !out.is_empty() && !latex_spaces_itself(atoms, i) {
                    out.push_str("\\ ");
                }
                i += 1;
            }
            Atom::Glyph { g, script: None } => {
                let base = glyph_latex(g);
                i += 1;
                let (subs, sups, limits, next) = take_scripts(atoms, i);
                i = next;
                // `\limits` is legal ONLY after an operator, and KaTeX errors
                // on it anywhere else — so it is emitted only where the base
                // is one this module recognised as such, even though the
                // GEOMETRY says "centred" for any centred pair.
                let forced = limits && is_big_operator(&base);
                push_prose_merged(&mut out, &base);
                if forced {
                    out.push_str("\\limits");
                }
                push_scripts(&mut out, &subs, &sups);
            }
            // A script with no base before it (a run that opens with one, or
            // a leftover after a fraction): emitted against an empty group, so
            // `^{2}` is still valid math rather than a bare `^`.
            Atom::Glyph {
                script: Some(_), ..
            } => {
                let (subs, sups, _, next) = take_scripts(atoms, i);
                i = next;
                out.push_str("{}");
                push_scripts(&mut out, &subs, &sups);
            }
            Atom::Frac { above, below } => {
                out.push_str(&format!(
                    "\\frac{{{}}}{{{}}}",
                    write_atoms(above),
                    write_atoms(below)
                ));
                i += 1;
                let (subs, sups, _, next) = take_scripts(atoms, i);
                i = next;
                push_scripts(&mut out, &subs, &sups);
            }
        }
    }
    out
}

/// Consume the maximal run of script atoms starting at `i`, returning the
/// subscript text, the superscript text, whether they were set as LIMITS, and
/// the index after them.
///
/// Both directions are collected in ONE pass because they interleave in `dx`
/// order and cannot be separated by position: `layout_math_list` centres an
/// upper and a lower limit on the same base, so `\sum_{k=1}^{n}`'s `n` sorts
/// between the `k` and the `1`. Splitting by `Script::up` and concatenating
/// each side in the order the atoms arrived — which is `dx` order within a
/// side, since [`mathrec::recover`] sorted them — puts each group back
/// together.
///
/// A space INSIDE a script run is kept (a subscript can be `i, j`), but a
/// space that ends it is not consumed: it belongs to the base line.
fn take_scripts(atoms: &[Atom<'_>], mut i: usize) -> (String, String, bool, usize) {
    let mut subs = String::new();
    let mut sups = String::new();
    let mut limits = false;
    let mut seen = false;
    while i < atoms.len() {
        match &atoms[i] {
            Atom::Glyph {
                g,
                script: Some(Script { up, limit }),
            } => {
                seen = true;
                limits |= *limit;
                let side = if *up { &mut sups } else { &mut subs };
                push_tok(side, &glyph_latex(g));
                i += 1;
            }
            // A space is absorbed whenever a further script follows it —
            // INCLUDING before the first one, which is not a corner case:
            // `layout_math_list` sets an integral's scripts at the base's own
            // advance, and for `∫` that advance is the DISPLAY variant's,
            // leaving a measured 4.008pt gap after the 7.980pt glyph actually
            // drawn. That is well over the space threshold, so `\int_{0}^{1}`
            // arrives here as base, space, script — and writing the space
            // would produce `\int\ {}_{0}^{1}`, an integral with its limits
            // detached and hung off an empty group.
            Atom::Space
                if matches!(
                    atoms.get(i + 1),
                    Some(Atom::Glyph {
                        script: Some(_),
                        ..
                    })
                ) =>
            {
                // Between two scripts the space is real (a limit reading
                // `k = 1`). Which SIDE it belongs to is not recoverable, so it
                // goes to whichever is open, preferring the subscript — where
                // a multi-token limit almost always is. Before the first
                // script there is no side yet and nothing to keep.
                if seen {
                    if subs.is_empty() && !sups.is_empty() {
                        sups.push_str("\\ ");
                    } else if !subs.is_empty() {
                        subs.push_str("\\ ");
                    }
                }
                i += 1;
            }
            _ => break,
        }
    }
    (subs, sups, limits, i)
}

/// Would LaTeX put this space in by itself?
///
/// **A recovered gap is not authored spacing.** SATySFi's math layout inserts
/// inter-atom spacing around a relation or a binary operator by exactly the
/// rules LaTeX uses, so by the time the glyphs are placed, the gap on either
/// side of a `+` is DERIVED information — and re-emitting it as `\ ` on top of
/// the space LaTeX will insert anyway renders the equation wider than the PDF.
/// `x + 1` must come out as `x+1`, which typesets as `x + 1`.
///
/// What is left over, and is kept, is the spacing LaTeX would NOT supply: the
/// gap between two ordinary atoms. That is not decoration either — it is the
/// only surviving trace of the word spaces inside a `text-in-math` run, which
/// `math_boxes_of_inline_boxes` turns into bare advance with no character
/// attached, so `${x \text!{ if and only if } y}` would otherwise concatenate
/// to `ifandonlyif`.
///
/// **Deliberately over-inclusive**, by whole Unicode blocks rather than by a
/// second symbol table. An atom wrongly called auto-spaced only DROPS a `\ `
/// that LaTeX may not replace, which is a hair too tight; one wrongly called
/// ordinary ADDS a space to one LaTeX already inserted, which is visibly
/// double. Erring toward the block is erring toward LaTeX's own judgement,
/// which is the reference here.
fn latex_spaces_itself(atoms: &[Atom<'_>], i: usize) -> bool {
    let side = |idx: Option<usize>| -> bool {
        match idx.and_then(|j| atoms.get(j)) {
            Some(Atom::Glyph { g, .. }) => g.text.chars().any(auto_spaced),
            // A fraction is an ordinary atom; so is a run boundary, which the
            // caller has already guarded against writing a leading space at.
            _ => false,
        }
    };
    side(i.checked_sub(1)) || side(Some(i + 1))
}

/// Whether LaTeX gives `c` spacing of its own — the Bin, Rel and Op classes.
///
/// By block, for the reason [`latex_spaces_itself`] gives:
///
/// - **Mathematical Operators** (U+2200–U+22FF) and **Supplemental
///   Mathematical Operators** (U+2A00–U+2AFF) are overwhelmingly Bin/Rel/Op;
///   the handful of Ord members (`∂ ∇ ∞ ∅ ∠`) are the acceptable direction of
///   error.
/// - **Arrows** (U+2190–U+21FF) and the long forms (U+27F0–U+27FF) are Rel.
/// - The ASCII and Latin-1 operators, which have no block of their own.
///
/// `pub(crate)` for [`crate::mathml`], which asks the identical question of
/// MathML Core's operator dictionary — Core's dictionary IS these classes, so
/// a second table would be a second chance to disagree with this one.
pub(crate) fn auto_spaced(c: char) -> bool {
    matches!(c,
        '+' | '-' | '=' | '<' | '>' | ':' | ',' | ';' | '*' | '/'
        | '\u{00AC}' | '\u{00B1}' | '\u{00D7}' | '\u{00F7}' | '\u{2212}'
        | '\u{2190}'..='\u{21FF}'
        | '\u{2200}'..='\u{22FF}'
        | '\u{27F0}'..='\u{27FF}'
        | '\u{2A00}'..='\u{2AFF}'
    )
}

/// Append `_{…}` and/or `^{…}`, in that order, skipping either when empty.
///
/// Always braced, even for one character: `x^{2}` and `x^2` render alike, but
/// `x^{12}` and `x^12` do not, and this cannot tell how many characters a
/// script's glyph record holds without looking.
fn push_scripts(out: &mut String, subs: &str, sups: &str) {
    if !subs.is_empty() {
        out.push_str(&format!("_{{{subs}}}"));
    }
    if !sups.is_empty() {
        out.push_str(&format!("^{{{sups}}}"));
    }
}

/// Whether `base` is one of the operators LaTeX allows `\limits` after.
///
/// Deliberately a fixed list rather than "starts with a backslash": `\limits`
/// after a non-operator is a hard error in KaTeX ("Limit controls must follow
/// a math operator"), so an unrecognised command must not get one. The list is
/// LaTeX's own `\mathop`-class large operators, which are exactly the commands
/// [`symbol_command`] can produce that take limits.
fn is_big_operator(base: &str) -> bool {
    const OPS: [&str; 15] = [
        "\\sum",
        "\\prod",
        "\\coprod",
        "\\int",
        "\\oint",
        "\\iint",
        "\\iiint",
        "\\bigcup",
        "\\bigcap",
        "\\bigsqcup",
        "\\biguplus",
        "\\bigvee",
        "\\bigwedge",
        "\\bigoplus",
        "\\bigotimes",
    ];
    OPS.contains(&base)
}

/// One glyph record's characters as LaTeX.
///
/// A record is not always one character — `math_boxes_of_inline_boxes` folds a
/// whole `text-in-math` run into one — so a record is first classified as
/// PROSE or as mathematics, and only the latter is mapped character by
/// character.
///
/// See [`is_prose_run`] for the classification; it is what lets `\text{…}`
/// come back at all, and what keeps a Japanese annotation inside an equation
/// out of math mode, where KaTeX warns about every character of it and sets
/// them all in italics with no inter-word spacing.
fn glyph_latex(g: &MathGlyph) -> String {
    // A combining mark must never reach `\text{…}`: KaTeX cannot lex a brace
    // with a diacritic on it and throws. Named marks become their accent
    // command; an unnamed one is DROPPED, which loses a mark but keeps the
    // equation renderable — see [`accent_command`].
    if g.text.chars().all(is_combining) {
        let mut out = String::new();
        for c in g.text.chars() {
            if let Some(accent) = accent_command(c) {
                push_tok(&mut out, &format!("{accent}{{}}"));
            }
        }
        return out;
    }
    if is_prose_run(&g.text) {
        return format!("\\text{{{}}}", text_escape(&g.text));
    }
    let mut out = String::new();
    for c in g.text.chars() {
        push_tok(&mut out, &char_latex(c));
    }
    out
}

/// Is this record ordinary PROSE that happened to be set inside an equation,
/// rather than mathematics?
///
/// Two signals, both structural rather than statistical:
///
/// - **it holds a space.** `math_boxes_of_inline_boxes` folds a whole
///   `text-in-math` `InnerString` into ONE glyph record, so `\text{if and only
///   if}` arrives as a single sixteen-character record — and nothing in
///   mathematics puts a space inside one atom. (The spaces BETWEEN records are
///   a different thing entirely and are gone by then; see
///   [`latex_spaces_itself`].)
/// - **it holds a character with no reading as mathematics** — a CJK
///   ideograph, kana, an accented Latin letter. Those reach an equation only
///   through `text-in-math`, and they arrive one record per character, which
///   is why the space test alone does not catch them.
///
/// A multi-character record that is neither — a run of digits, say — stays
/// mathematics, so `x^{12}` is not quietly turned into `x^{\text{12}}`.
fn is_prose_run(text: &str) -> bool {
    text.contains(' ') || text.chars().any(needs_text_mode)
}

/// A character that has no reading in math mode at all.
///
/// Everything with a name in [`symbol_command`], everything Unicode encodes as
/// a styled mathematical letter, the Greek block and plain ASCII are
/// mathematics. What is left is prose: CJK, kana, Hangul, accented Latin,
/// anything a document set with `text-in-math` — for which KaTeX emits a
/// `unicodeTextInMathMode` warning and renders in an italic math face with the
/// inter-atom spacing of a variable, which is not what the PDF shows.
fn needs_text_mode(c: char) -> bool {
    if c.is_ascii() {
        return false;
    }
    // A combining mark is neither prose nor an atom: it is handled ahead of
    // this by [`glyph_latex`], and must not be allowed to reach `\text{…}`
    // even in a mixed record, where it would break the whole formula.
    if is_combining(c) {
        return false;
    }
    let base = rustyfi_backend::math_alphanumeric_base(c);
    if base.is_some() || symbol_command(c).is_some() {
        return false;
    }
    // Greek and Coptic — the letters a formula names directly rather than
    // through a `\alpha`-style command.
    !matches!(c as u32, 0x370..=0x3FF)
}

/// The contents of a `\text{…}` group.
///
/// A different escape from [`escape`]: inside `\text` the characters are
/// ordinary text, so `-` and `^` are themselves and only LaTeX's own ten
/// reserved characters need spelling out. `\textbackslash` rather than
/// `\backslash`, which is a math-mode command.
fn text_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '$' => out.push_str("\\$"),
            '&' => out.push_str("\\&"),
            '#' => out.push_str("\\#"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            '^' => out.push_str("\\textasciicircum{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            _ => out.push(c),
        }
    }
    out
}

/// [`push_tok`], plus: fold this `\text{…}` group into the one immediately
/// before it, when both are.
///
/// A Japanese annotation inside an equation arrives one record per character
/// (`text-in-math` folds a Latin run into one record, but the box stream
/// splits CJK per character long before that), so `\underset{\text!{運動エネ
/// ルギーを表す}}` would otherwise come out as ten separate `\text{}` groups —
/// 60 bytes of markup around 30 of content, in a format whose whole claim is
/// that the raw file is legible. It renders identically either way; this is
/// about what the file looks like.
///
/// Structural rather than a `"}\\text{"` string replacement, which would also
/// splice `\frac{a}{b}\text{x}` into `\frac{a}{bx}` — a different equation.
/// The merge fires only when the token just written was itself a whole
/// `\text{…}` group, which is a fact this function has and a regex does not.
fn push_prose_merged(out: &mut String, tok: &str) {
    const OPEN: &str = "\\text{";
    let both_prose = tok.starts_with(OPEN)
        && tok.ends_with('}')
        && out.ends_with('}')
        && last_group_is_text(out);
    if both_prose {
        out.pop();
        out.push_str(&tok[OPEN.len()..]);
        return;
    }
    push_tok(out, tok);
}

/// Does `out` end with a complete, unnested `\text{…}` group?
///
/// Unnested is the whole test: `\text` groups this module writes never contain
/// a brace of their own ([`text_escape`] spells `{` as `\{`), so the group
/// runs from the LAST `\text{` to the end and is a match exactly when no `{`
/// or `}` occurs inside it.
fn last_group_is_text(out: &str) -> bool {
    let Some(at) = out.rfind("\\text{") else {
        return false;
    };
    let inner = &out[at + "\\text{".len()..out.len() - 1];
    !inner.contains('{') && !inner.contains('}')
}

/// Append `tok`, inserting the space that keeps a control word from swallowing
/// what follows it.
///
/// **The bug this exists to prevent, measured on `latexcmds`' manual.** A
/// LaTeX control word runs to the first non-letter, so
/// `\partial` + `t` concatenates to `\partialt` — an undefined command, which
/// KaTeX renders as a red error and LaTeX refuses outright. The Schrödinger
/// equation in that manual produced `\frac{\partial}{\partialt}` and
/// `\partialx_{2}`; both are one space away from correct and neither is
/// visible in the source without knowing the rule.
///
/// Every concatenation in this module goes through here, rather than the
/// obvious "put a space after each command", because a trailing space after
/// every symbol would double the size of a dense formula and put a stray space
/// before `_`, `^` and `}` — all of which are ordinary characters to the token
/// scanner but not to a reader diffing the output.
fn push_tok(out: &mut String, tok: &str) {
    if ends_in_control_word(out) && tok.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.push(' ');
    }
    out.push_str(tok);
}

/// Does `s` end inside a `\command` name — i.e. would one more letter extend
/// it rather than follow it?
fn ends_in_control_word(s: &str) -> bool {
    let letters = s
        .bytes()
        .rev()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    letters > 0 && s.len() > letters && s.as_bytes()[s.len() - letters - 1] == b'\\'
}

/// The LaTeX accent command a combining mark stands for, if this module knows
/// one.
///
/// **The single largest defect this module had, measured**: 471 of 1358
/// emitted formulas failed real KaTeX outright, and 233 of `azmath`'s 328 math
/// spans — its whole accent chapter — rendered as red errors.
///
/// The mechanism is worth recording, because nothing about it is visible in
/// the output. SATySFi's math places an accent as its OWN glyph record holding
/// a lone combining mark (U+0300–U+036F), positioned over the character it
/// accents. Such a record is not ASCII, has no `symbol_command` and is not a
/// styled letter, so [`needs_text_mode`] correctly concluded "not
/// mathematics" and [`glyph_latex`] wrapped it: `\text{̂}`. KaTeX's lexer then
/// glues the `{` to the combining mark — a brace with a diacritic on it is not
/// a token — and throws. lualatex ACCEPTS the same input and silently drops
/// the glyph, so this is precisely the gap where KaTeX is narrower than LaTeX
/// and a `lualatex` check cannot see it.
///
/// Mapped to the accent command rather than dropped, and applied to an EMPTY
/// group (`\hat{}`) rather than to the preceding atom. That is deliberate:
/// the mark arrives as a separate record with its own position, and this
/// module's recovery has no notion of "the atom before me" that would survive
/// a script or a fraction boundary — so `\hat{x}` is not reconstructible here
/// without guessing. `x\hat{}` renders the base and the accent side by side
/// rather than stacked: visibly approximate, which is the honest failure, and
/// it PARSES, which is the point.
///
/// Anything in the range this table does not name falls through to
/// [`needs_text_mode`] as before — but as a combining mark it would break
/// KaTeX again, so [`glyph_latex`] drops an unnamed one instead.
fn accent_command(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{300}' => "\\grave",
        '\u{301}' => "\\acute",
        '\u{302}' => "\\hat",
        '\u{303}' => "\\tilde",
        '\u{304}' => "\\bar",
        '\u{306}' => "\\breve",
        '\u{307}' => "\\dot",
        '\u{308}' => "\\ddot",
        '\u{30A}' => "\\mathring",
        '\u{30C}' => "\\check",
        '\u{20D7}' => "\\vec",
        _ => return None,
    })
}

/// Is `c` a combining mark — a character that composes with the one before it
/// rather than standing on its own?
///
/// The Unicode "Combining Diacritical Marks" block plus the two combining
/// ranges a math font actually reaches. Used to keep a mark out of `\text{…}`,
/// where KaTeX cannot lex it; see [`accent_command`].
///
/// `pub(crate)` for [`crate::mathml`], which needs the same set for the
/// opposite purpose — a lone combining mark is exactly what it turns into an
/// `<mover accent="true">`, which is the shape this module cannot write.
pub(crate) fn is_combining(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F      // Combining Diacritical Marks
        | 0x20D0..=0x20F0    // Combining Diacritical Marks for Symbols
        | 0xFE20..=0xFE2F    // Combining Half Marks
    )
}

/// One character as LaTeX: its style wrapper (when Unicode encoded one), then
/// its command or its escaped self.
fn char_latex(c: char) -> String {
    if let Some(accent) = accent_command(c) {
        return format!("{accent}{{}}");
    }
    let base = rustyfi_backend::math_alphanumeric_base(c).unwrap_or(c);
    let body = symbol_command(base)
        .map(str::to_string)
        .unwrap_or_else(|| escape(base));
    match style_wrapper(c) {
        // A styled command (`\mathbb{\alpha}`) is not what anyone means, and
        // KaTeX rejects several such combinations outright; only a plain
        // letter or digit takes a wrapper.
        Some(cmd) if symbol_command(base).is_none() => format!("{cmd}{{{body}}}"),
        _ => body,
    }
}

/// The `\math…` wrapper Unicode's Mathematical Alphanumeric Symbols block
/// encoded into `c`, if any.
///
/// This is the one piece of styling that genuinely survives layout, and it
/// survives because SATySFi's math writes it into the CODEPOINT: `${\bold{R}}`
/// is laid out as `𝐑` U+1D411, not as `R` with a bold flag, so the style is
/// still there to read. `rustyfi_backend::math_alphanumeric_base` takes the
/// character back to `R`; this says which of the fourteen alphabets it came
/// from.
///
/// The block is runs of fixed stride, in Unicode's own order — the same
/// arithmetic `math_alphanumeric_base` documents and
/// `alphanumeric_block_strides_tile_the_block` pins, read for the run INDEX
/// here rather than the offset within it. Italic returns `None`: math mode
/// already italicises a letter, so `\mathit{x}` would be a no-op that only
/// made the output longer.
fn style_wrapper(c: char) -> Option<&'static str> {
    let u = c as u32;
    // The Letterlike Symbols holes are the styled letters that got their own
    // codepoints outside the block (`ℝ`, `ℋ`, `ℭ`, …). They carry the same
    // styles and must reach the same wrappers, or `ℝ` would come out as a
    // bare `R` while `𝔸` came out as `\mathbb{A}`.
    if let Some(w) = letterlike_style(u) {
        return Some(w);
    }
    if !(0x1D400..=0x1D7FF).contains(&u) {
        return None;
    }
    // 13 alphabetic runs of 52, in Unicode's order.
    if u < 0x1D6A4 {
        return match (u - 0x1D400) / 52 {
            0 => Some("\\mathbf"),
            1 => None, // italic — math mode's own default
            2 => Some("\\boldsymbol"),
            3 | 4 => Some("\\mathcal"),
            5 | 7 => Some("\\mathfrak"),
            6 => Some("\\mathbb"),
            8..=11 => Some("\\mathsf"),
            12 => Some("\\mathtt"),
            _ => None,
        };
    }
    // The two dotless letters, then 5 Greek runs of 58, then the digammas.
    if (0x1D6A8..0x1D7CA).contains(&u) {
        return match (u - 0x1D6A8) / 58 {
            0 => Some("\\mathbf"),
            1 => None,
            2 => Some("\\boldsymbol"),
            3 | 4 => Some("\\mathsf"),
            _ => None,
        };
    }
    // 5 digit runs of 10.
    if (0x1D7CE..=0x1D7FF).contains(&u) {
        return match (u - 0x1D7CE) / 10 {
            0 => Some("\\mathbf"),
            1 => Some("\\mathbb"),
            2 | 3 => Some("\\mathsf"),
            4 => Some("\\mathtt"),
            _ => None,
        };
    }
    None
}

/// The style of a Letterlike Symbols character that stands in for a hole in
/// the Mathematical Alphanumeric Symbols block — the set
/// `math_alphanumeric_base`'s own `letterlike` table covers.
fn letterlike_style(u: u32) -> Option<&'static str> {
    Some(match u {
        // Script capitals and smalls (`ℬ ℰ ℱ ℋ ℐ ℒ ℳ ℛ ℯ ℊ ℴ ℓ`).
        0x212C | 0x2130 | 0x2131 | 0x210B | 0x2110 | 0x2112 | 0x2133 | 0x211B | 0x212F
        | 0x210A | 0x2134 | 0x2113 => "\\mathcal",
        // Fraktur (`ℭ ℌ ℑ ℜ ℨ`).
        0x212D | 0x210C | 0x2111 | 0x211C | 0x2128 => "\\mathfrak",
        // Double-struck (`ℂ ℍ ℕ ℙ ℚ ℝ ℤ`, and the Greek/summation forms).
        0x2102 | 0x210D | 0x2115 | 0x2119 | 0x211A | 0x211D | 0x2124 | 0x213C | 0x213D
        | 0x213E | 0x213F | 0x2140 => "\\mathbb",
        // `ℎ` is plain italic, and `ℏ`/`ℹ` are their own characters rather
        // than styled ones — none of the three takes a wrapper.
        _ => return None,
    })
}

/// `c`'s LaTeX command, when it has one.
///
/// Restricted to what KaTeX supports without a package, since a command
/// MathJax knows and KaTeX does not would render on one reader's page and
/// throw a parse error on another's. Everything absent from this table falls
/// through to [`escape`] and renders as the character itself, which is the
/// honest outcome for a symbol whose name this module does not know.
fn symbol_command(c: char) -> Option<&'static str> {
    Some(match c {
        // ---- Greek, lower case -------------------------------------------
        '\u{3B1}' => "\\alpha",
        '\u{3B2}' => "\\beta",
        '\u{3B3}' => "\\gamma",
        '\u{3B4}' => "\\delta",
        '\u{3B5}' => "\\varepsilon",
        '\u{3F5}' => "\\epsilon",
        '\u{3B6}' => "\\zeta",
        '\u{3B7}' => "\\eta",
        '\u{3B8}' => "\\theta",
        '\u{3D1}' => "\\vartheta",
        '\u{3B9}' => "\\iota",
        '\u{3BA}' => "\\kappa",
        '\u{3F0}' => "\\varkappa",
        '\u{3BB}' => "\\lambda",
        '\u{3BC}' => "\\mu",
        '\u{3BD}' => "\\nu",
        '\u{3BE}' => "\\xi",
        '\u{3C0}' => "\\pi",
        '\u{3D6}' => "\\varpi",
        '\u{3C1}' => "\\rho",
        '\u{3F1}' => "\\varrho",
        '\u{3C3}' => "\\sigma",
        '\u{3C2}' => "\\varsigma",
        '\u{3C4}' => "\\tau",
        '\u{3C5}' => "\\upsilon",
        '\u{3C6}' => "\\varphi",
        '\u{3D5}' => "\\phi",
        '\u{3C7}' => "\\chi",
        '\u{3C8}' => "\\psi",
        '\u{3C9}' => "\\omega",
        // ---- Greek, upper case -------------------------------------------
        '\u{393}' => "\\Gamma",
        '\u{394}' => "\\Delta",
        '\u{398}' => "\\Theta",
        '\u{3F4}' => "\\Theta",
        '\u{39B}' => "\\Lambda",
        '\u{39E}' => "\\Xi",
        '\u{3A0}' => "\\Pi",
        '\u{3A3}' => "\\Sigma",
        '\u{3A5}' => "\\Upsilon",
        '\u{3A6}' => "\\Phi",
        '\u{3A8}' => "\\Psi",
        '\u{3A9}' => "\\Omega",
        // ---- Large operators ---------------------------------------------
        '\u{2211}' => "\\sum",
        '\u{220F}' => "\\prod",
        '\u{2210}' => "\\coprod",
        '\u{222B}' => "\\int",
        '\u{222C}' => "\\iint",
        '\u{222D}' => "\\iiint",
        '\u{222E}' => "\\oint",
        '\u{22C3}' => "\\bigcup",
        '\u{22C2}' => "\\bigcap",
        '\u{2A06}' => "\\bigsqcup",
        '\u{2A04}' => "\\biguplus",
        '\u{22C1}' => "\\bigvee",
        '\u{22C0}' => "\\bigwedge",
        '\u{2A01}' => "\\bigoplus",
        '\u{2A02}' => "\\bigotimes",
        // ---- Binary operators --------------------------------------------
        '\u{00B1}' => "\\pm",
        '\u{2213}' => "\\mp",
        '\u{00D7}' => "\\times",
        '\u{00F7}' => "\\div",
        '\u{22C5}' => "\\cdot",
        '\u{2218}' => "\\circ",
        '\u{2219}' => "\\bullet",
        '\u{2295}' => "\\oplus",
        '\u{2296}' => "\\ominus",
        '\u{2297}' => "\\otimes",
        '\u{2298}' => "\\oslash",
        '\u{2299}' => "\\odot",
        '\u{222A}' => "\\cup",
        '\u{2229}' => "\\cap",
        '\u{228E}' => "\\uplus",
        '\u{2294}' => "\\sqcup",
        '\u{2293}' => "\\sqcap",
        '\u{2227}' => "\\wedge",
        '\u{2228}' => "\\vee",
        '\u{2216}' => "\\setminus",
        '\u{2217}' => "\\ast",
        '\u{2242}' => "\\eqsim",
        // ---- Relations ----------------------------------------------------
        '\u{2264}' => "\\leq",
        '\u{2265}' => "\\geq",
        '\u{2260}' => "\\neq",
        '\u{2248}' => "\\approx",
        '\u{2261}' => "\\equiv",
        '\u{223C}' => "\\sim",
        '\u{2243}' => "\\simeq",
        '\u{2245}' => "\\cong",
        '\u{221D}' => "\\propto",
        '\u{226A}' => "\\ll",
        '\u{226B}' => "\\gg",
        '\u{2208}' => "\\in",
        '\u{2209}' => "\\notin",
        '\u{220B}' => "\\ni",
        '\u{2282}' => "\\subset",
        '\u{2283}' => "\\supset",
        '\u{2286}' => "\\subseteq",
        '\u{2287}' => "\\supseteq",
        '\u{228A}' => "\\subsetneq",
        '\u{228B}' => "\\supsetneq",
        '\u{22A2}' => "\\vdash",
        '\u{22A8}' => "\\models",
        '\u{22A5}' => "\\perp",
        '\u{2225}' => "\\parallel",
        '\u{2223}' => "\\mid",
        '\u{225C}' => "\\triangleq",
        '\u{2250}' => "\\doteq",
        // ---- Arrows -------------------------------------------------------
        '\u{2192}' => "\\to",
        '\u{2190}' => "\\leftarrow",
        '\u{2194}' => "\\leftrightarrow",
        '\u{21D2}' => "\\Rightarrow",
        '\u{21D0}' => "\\Leftarrow",
        '\u{21D4}' => "\\Leftrightarrow",
        '\u{21A6}' => "\\mapsto",
        '\u{27F6}' => "\\longrightarrow",
        '\u{27F5}' => "\\longleftarrow",
        '\u{27F9}' => "\\Longrightarrow",
        '\u{2191}' => "\\uparrow",
        '\u{2193}' => "\\downarrow",
        // ---- Miscellaneous symbols ----------------------------------------
        '\u{221E}' => "\\infty",
        '\u{2202}' => "\\partial",
        '\u{2207}' => "\\nabla",
        '\u{2200}' => "\\forall",
        '\u{2203}' => "\\exists",
        '\u{2204}' => "\\nexists",
        '\u{00AC}' => "\\neg",
        '\u{2205}' => "\\emptyset",
        '\u{2135}' => "\\aleph",
        '\u{2118}' => "\\wp",
        '\u{2220}' => "\\angle",
        '\u{25B3}' => "\\triangle",
        '\u{25A1}' => "\\square",
        '\u{266D}' => "\\flat",
        '\u{266F}' => "\\sharp",
        '\u{266E}' => "\\natural",
        '\u{2032}' => "'",
        '\u{2033}' => "''",
        '\u{2026}' => "\\ldots",
        '\u{22EF}' => "\\cdots",
        '\u{22EE}' => "\\vdots",
        '\u{22F1}' => "\\ddots",
        '\u{2234}' => "\\therefore",
        '\u{2235}' => "\\because",
        '\u{2212}' => "-",
        '\u{2044}' => "/",
        // ---- Delimiters that are not their own ASCII character ------------
        '\u{27E8}' => "\\langle",
        '\u{27E9}' => "\\rangle",
        '\u{2308}' => "\\lceil",
        '\u{2309}' => "\\rceil",
        '\u{230A}' => "\\lfloor",
        '\u{230B}' => "\\rfloor",
        '\u{2016}' => "\\|",
        // The dotless letters, which `math_alphanumeric_base` maps to Latin
        // Extended-A rather than to `i`/`j` — deliberately, since a dotted `i`
        // where the author wrote a dotless one is wrong rather than unstyled.
        '\u{131}' => "\\imath",
        '\u{237}' => "\\jmath",
        '\u{127}' => "\\hbar",
        _ => return None,
    })
}

/// A character with no command of its own, made safe for math mode.
///
/// The ten characters LaTeX reserves are the whole of the danger here, and
/// each has an established math-mode spelling. Anything else — a letter, a
/// digit, an ordinary punctuation mark, a CJK character that wandered into an
/// equation — is passed through as itself: KaTeX renders an unknown
/// non-reserved character literally, which is exactly what the document set.
fn escape(c: char) -> String {
    match c {
        '\\' => "\\backslash".to_string(),
        '{' => "\\{".to_string(),
        '}' => "\\}".to_string(),
        '$' => "\\$".to_string(),
        '&' => "\\&".to_string(),
        '#' => "\\#".to_string(),
        '%' => "\\%".to_string(),
        '_' => "\\_".to_string(),
        '^' => "\\hat{}".to_string(),
        '~' => "\\sim".to_string(),
        _ => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathrec::tests::glyph;
    use rustyfi_backend::{Closing, Color, GraphicsElem, Length, Path, PathSeg, Subpath};

    fn latex(glyphs: &[MathGlyph]) -> String {
        math_latex(glyphs, &[])
    }

    #[test]
    fn a_script_becomes_a_braced_superscript() {
        assert_eq!(
            latex(&[glyph("x", 0.0, 0.0, 10.0), glyph("2", 5.0, 4.0, 7.0)]),
            "x^{2}"
        );
    }

    /// The failure this exists to prevent: LaTeX permits ONE subscript per
    /// base, so a multi-glyph limit emitted one atom at a time produces
    /// `\sum_{k}_{=}_{1}`, which KaTeX refuses to render at all.
    #[test]
    fn a_multi_glyph_limit_becomes_one_subscript_group() {
        // `\sum_{k=1}^{n}`: the limits are centred on the operator, and the
        // superscript sorts BETWEEN the subscript's glyphs in `dx` order,
        // which is what a naive per-atom emitter gets wrong.
        let out = latex(&[
            glyph("\u{2211}", 0.0, 0.0, 12.0),
            glyph("k", 0.5, -6.0, 8.0),
            glyph("n", 1.5, 7.0, 8.0),
            glyph("=", 2.0, -6.0, 8.0),
            glyph("1", 3.5, -6.0, 8.0),
        ]);
        assert_eq!(out, "\\sum\\limits_{k=1}^{n}");
        assert_eq!(out.matches('_').count(), 1, "one subscript only: {out}");
    }

    /// `\limits` after anything that is not an operator is a hard KaTeX error,
    /// so a centred script on an ordinary letter must not get one.
    #[test]
    fn limits_are_only_emitted_after_a_recognised_operator() {
        let out = latex(&[glyph("R", 0.0, 0.0, 12.0), glyph("k", 0.5, -6.0, 8.0)]);
        assert!(!out.contains("\\limits"), "{out}");
        assert_eq!(out, "R_{k}");
    }

    /// Unicode encodes the alphabet in the codepoint, so this one piece of
    /// styling really is recoverable — `𝔸` is not an `A`.
    #[test]
    fn a_styled_letter_recovers_its_alphabet() {
        assert_eq!(latex(&[glyph("\u{1D538}", 0.0, 0.0, 10.0)]), "\\mathbb{A}");
        assert_eq!(latex(&[glyph("\u{1D504}", 0.0, 0.0, 10.0)]), "\\mathfrak{A}");
        assert_eq!(latex(&[glyph("\u{1D400}", 0.0, 0.0, 10.0)]), "\\mathbf{A}");
        // `ℝ` lives outside the block but means the same thing.
        assert_eq!(latex(&[glyph("\u{211D}", 0.0, 0.0, 10.0)]), "\\mathbb{R}");
        // Math italic is math mode's own default; a wrapper would be noise.
        assert_eq!(latex(&[glyph("\u{1D434}", 0.0, 0.0, 10.0)]), "A");
        // A styled GREEK letter keeps its command and takes no wrapper —
        // `\mathbb{\alpha}` is not a thing KaTeX will render.
        assert_eq!(latex(&[glyph("\u{1D6FC}", 0.0, 0.0, 10.0)]), "\\alpha");
    }

    /// A fraction bar is the one structure the box stream still carries, so it
    /// is the one that comes back as a real construct.
    #[test]
    fn a_fraction_bar_becomes_a_frac() {
        let bar = GraphicsElem::Fill(
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: (Length(0.0), Length(4.0)),
                    segs: vec![
                        PathSeg::Line((Length(20.0), Length(4.0))),
                        PathSeg::Line((Length(20.0), Length(4.5))),
                        PathSeg::Line((Length(0.0), Length(4.5))),
                    ],
                    closing: Closing::Line,
                }],
            },
        );
        let out = math_latex(
            &[
                glyph("a", 2.0, 8.0, 10.0),
                glyph("+", 7.0, 8.0, 10.0),
                glyph("b", 12.0, 8.0, 10.0),
                glyph("c", 8.0, -4.0, 10.0),
            ],
            &[bar],
        );
        assert_eq!(out, "\\frac{a+b}{c}");
    }

    /// A reserved character must render as itself, not end the formula or
    /// silently vanish. `_` is the one that would otherwise turn the rest of
    /// the equation into a subscript.
    #[test]
    fn reserved_characters_are_escaped_rather_than_dropped() {
        let out = latex(&[
            glyph("a", 0.0, 0.0, 10.0),
            glyph("_", 5.0, 0.0, 10.0),
            glyph("%", 10.0, 0.0, 10.0),
            glyph("{", 15.0, 0.0, 10.0),
        ]);
        assert_eq!(out, "a\\_\\%\\{");
    }

    /// An empty run writes nothing, so the caller never emits a bare `$$`.
    #[test]
    fn an_empty_run_produces_no_latex() {
        assert_eq!(latex(&[]), "");
    }

    /// The gap either side of a binary operator is spacing SATySFi's layout
    /// inserted by LaTeX's own rules, so re-emitting it would double it. The
    /// gap between two ordinary atoms is not, and is the only trace left of a
    /// word space inside a `text-in-math` run.
    #[test]
    fn latex_supplies_its_own_operator_spacing_but_not_a_word_space() {
        // `x + 1`, with real gaps around the `+` — the same fixture the
        // Unicode writer renders as `x + 1`.
        let spaced = latex(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("+", 12.0, 0.0, 10.0),
            glyph("1", 20.0, 0.0, 10.0),
        ]);
        assert_eq!(spaced, "x+1", "operator spacing must not be doubled");

        // Two ordinary runs with a measured gap: nothing in LaTeX would put
        // that space back, so it has to be written.
        let words = latex(&[
            glyph("if", 0.0, 0.0, 10.0),
            glyph("and", 12.0, 0.0, 10.0),
        ]);
        assert_eq!(words, "if\\ and");
    }

    /// A control word runs to the first non-letter, so a command followed by a
    /// variable name concatenates into an undefined command. Found in
    /// `latexcmds`' Schrödinger equation, which produced `\partialt`.
    #[test]
    fn a_command_followed_by_a_letter_gets_the_space_it_needs() {
        let out = latex(&[
            glyph("\u{2202}", 0.0, 0.0, 10.0),
            glyph("t", 5.0, 0.0, 10.0),
        ]);
        assert_eq!(out, "\\partial t");
        // …and only where it is needed: a command before a non-letter, or two
        // commands in a row, must not grow a space that changes nothing but
        // the size of the output.
        assert_eq!(
            latex(&[
                glyph("\u{3B1}", 0.0, 0.0, 10.0),
                glyph("\u{3B2}", 5.0, 0.0, 10.0),
            ]),
            "\\alpha\\beta"
        );
        assert_eq!(
            latex(&[glyph("x", 0.0, 0.0, 10.0), glyph("t", 5.0, 0.0, 10.0)]),
            "xt"
        );
        // The space must reach inside a script group too, which is built
        // separately from the base line.
        let scripted = latex(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("\u{2202}", 5.0, 4.0, 7.0),
            glyph("t", 8.0, 4.0, 7.0),
        ]);
        assert_eq!(scripted, "x^{\\partial t}");
    }

    /// Prose that was set inside an equation comes back as `\text{…}`, not as
    /// a row of italic math atoms. Two signals reach it, and both are
    /// exercised: a folded `text-in-math` run holds a space, and a CJK
    /// annotation arrives one character per record and holds none.
    #[test]
    fn prose_inside_an_equation_comes_back_as_text() {
        // `math_boxes_of_inline_boxes` folds a whole run into one record.
        assert_eq!(
            latex(&[glyph("if and only if", 0.0, 0.0, 10.0)]),
            "\\text{if and only if}"
        );
        // One CJK character per record — the shape `latexcmds`' own
        // `\underset{\text!{運動エネルギーを表す}}` actually produces.
        assert_eq!(
            latex(&[glyph("\u{904B}", 0.0, 0.0, 10.0)]),
            "\\text{\u{904B}}"
        );
        // A multi-character record that is NOT prose stays mathematics, or
        // `x^{12}` would quietly become `x^{\text{12}}`.
        assert_eq!(latex(&[glyph("12", 0.0, 0.0, 10.0)]), "12");
        // A reserved character inside a text group takes the text-mode
        // spelling, not the math-mode one.
        assert_eq!(
            latex(&[glyph("a & b", 0.0, 0.0, 10.0)]),
            "\\text{a \\& b}"
        );
    }

    /// Consecutive prose records fold into ONE `\text{}`. CJK reaches an
    /// equation one character per record, so without this a ten-character
    /// annotation is ten groups and 60 bytes of markup around 30 of content.
    #[test]
    fn adjacent_prose_records_merge_into_one_text_group() {
        let out = latex(&[
            glyph("\u{904B}", 0.0, 0.0, 10.0),
            glyph("\u{52D5}", 5.0, 0.0, 10.0),
            glyph("\u{91CF}", 10.0, 0.0, 10.0),
        ]);
        assert_eq!(out, "\\text{\u{904B}\u{52D5}\u{91CF}}");

        // …but a `}` that closes something else must NOT be reopened: a
        // string-level `"}\\text{"` replacement would splice this fraction's
        // denominator into the annotation and silently change the equation.
        let after_frac = latex(&[
            glyph("x", 0.0, 0.0, 10.0),
            glyph("\u{904B}", 5.0, 0.0, 10.0),
        ]);
        assert_eq!(after_frac, "x\\text{\u{904B}}");
        assert!(last_group_is_text("a\\text{b}"));
        assert!(!last_group_is_text("\\frac{a}{b}"));
        assert!(!last_group_is_text("\\text{a}x"));
    }

    /// `layout_math_list` sets an integral's scripts at the DISPLAY variant's
    /// advance, which leaves a gap wide enough to be read as a space between
    /// the operator and its own subscript. Writing that space detaches the
    /// limits and hangs them off an empty group.
    #[test]
    fn a_gap_before_a_script_does_not_detach_it() {
        let out = latex(&[
            // Base 12pt: `∫` is 6.0 wide here, and the script starts at 10.0,
            // a 4.0 gap — over the 2.64 threshold, so `recover` calls it a
            // space.
            glyph("\u{222B}", 0.0, 0.0, 12.0),
            glyph("0", 10.0, -5.0, 8.0),
            glyph("1", 10.5, 6.0, 8.0),
        ]);
        assert_eq!(out, "\\int_{0}^{1}");
        assert!(!out.contains("{}"), "the limits came off the operator: {out}");
        // Set BESIDE the operator rather than centred on it, so no `\limits`.
        assert!(!out.contains("\\limits"), "{out}");
    }
}
