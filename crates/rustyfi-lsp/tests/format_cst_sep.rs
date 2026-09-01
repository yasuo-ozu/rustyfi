//! The separation table, validated exhaustively against the corpus vocabulary.
//!
//! # The hole this closes
//!
//! `format_cst`'s safety argument is that every output byte is copied from a
//! byte range of the input, so no token can be added, dropped or reordered. It
//! has exactly one hole: two adjacent copied ranges written with **no separator
//! between them** can lex as a *single* token. `:` `:` becomes `::`; `1` `pt`
//! becomes one length; `&` `&` becomes one binop (which matters for staging,
//! where `&&x` is a quote of a quote); `<` `[` becomes `BPath`.
//! [`sep::must_separate`] is what stands in that hole, and before this file it
//! had never been checked against anything but the table of pairs it was written
//! from.
//!
//! Note the sign of the error, because it decides every judgement call here: a
//! false `true` costs one unnecessary space, a false `false` corrupts a
//! document. So the only direction asserted is
//!
//! > `must_separate(p, n) == false` **implies** writing `p` and `n` adjacently
//! > keeps them two tokens.
//!
//! Over-separation is measured and reported as a quality number, never as a
//! failure.
//!
//! # What "keeps them two tokens" is checked to mean
//!
//! For a pair `(p, n)` placed at byte offset `join` in some surrounding text,
//! the property is *local to the join*:
//!
//! > the lexer produces a token boundary exactly at `join`, and the tokens
//! > ending at or before `join` are the same ones it produced without `n`.
//!
//! That is precisely "no token was merged across the join, and none appeared or
//! vanished before it". It deliberately says nothing about the tokens *after*
//! the join: those are `n`'s own lexing, which in a real document continues into
//! the rest of the file anyway, and `format_cst`'s verifier re-lexes the whole
//! output regardless. Checking the join instead of the whole concatenation is
//! also what makes the sweep possible at all — `)` alone does not lex in program
//! mode, `}` does not either, and demanding that `p` and `n` each lex standalone
//! would skip most of the vocabulary (see "skips", below).
//!
//! # The context, and why there are five of them
//!
//! A pair must be tested where it can actually be adjacent. SATySFi's lexer is a
//! mode stack, and most spellings are legal in only one mode: `}` only closes an
//! inline-text area, `_` is a wildcard in program mode and a subscript in math.
//! So every spelling is placed after the shortest prefix that establishes the
//! mode it needs ([`CONTEXTS`]), and the *mode at the join* is then measured with
//! a canary rather than assumed.
//!
//! Only pairs whose **mode at the join is program mode** are asserted on, and
//! that is not a convenience — it is [`sep::must_separate`]'s domain. Inside a
//! text area *everything* fuses (`is_str_char` swallows most of ASCII), so no
//! character-class rule can be sound there, and none is needed: a text area is
//! copied through as one `Doc::Verbatim` whose interior the renderer never
//! re-spaces. That precondition is load-bearing enough to be pinned by a test of
//! its own, [`the_table_is_unsound_inside_a_text_area_which_is_why_its_domain_is_program_mode`].
//!
//! # Exhaustive, and where it is reduced
//!
//! - **Soundness** (the assertion) is exhaustive over the real corpus
//!   vocabulary: every ordered pair of the distinct token *spellings* the corpus
//!   contains — the source text a span covers, which is what the formatter
//!   actually copies — for which `must_separate` answers `false`. Nothing is
//!   sampled and nothing is canonicalised.
//! - **Mutation-tested**, because a generated test is exactly the kind that can
//!   stay green with the rule it checks gutted. Deleting each of `sep.rs`'s three
//!   arms in turn fails this file and names concrete pairs: without the
//!   `is_opsymbol` arm, 12861 pairs in 215 classes including `&` ++ `&` (the
//!   staging hazard); without [`sep::FUSED_DELIMITERS`], 1.35 M pairs including
//!   `(` ++ `|`, `<` ++ `[`, `]` ++ `>`, `|` ++ `)`; without the word-glue arms,
//!   26799 pairs including `+` ++ `a`, `100.` ++ `0` and `\fbox` ++ `@import: …`.
//! - **Over-separation** (the quality number) is measured over a
//!   character-class reduction of the same vocabulary, because it lives on the
//!   other ~90% of the cross product and paying for it exhaustively would cost
//!   minutes. The reduction maps each character to a representative of its
//!   lexer-visible class (see [`canon`]) and collapses runs; a spelling whose
//!   canonical form does not behave like it does is kept verbatim as well, so
//!   the reduction reports its own casualties instead of hiding them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rustyfi_syntax::{lex_partial, RustyfiVersion, Token};

#[path = "../src/format_cst/sep.rs"]
mod sep;

// ---------------------------------------------------------------------------
// The corpus, and the vocabulary it contains
// ---------------------------------------------------------------------------

/// Same collection helpers as `tests/format.rs:546-592` and
/// `tests/format_cst_identity.rs`, for the same 162 + 47 files.
fn corpus(dirs: &[&str]) -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    for d in dirs {
        collect(&root.join(d), &mut out);
    }
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|s| s.to_str()),
            Some("saty" | "satyh" | "satyg")
        ) {
            out.push(p);
        }
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

/// Every distinct token *spelling* in the bundled corpus, under both
/// generations.
///
/// Both corpora are lexed under both `RustyfiVersion`s, not each under its own:
/// the formatter picks a generation from the buffer it is handed, either lexer
/// may be pointed at either file, and the two disagree about real spellings
/// (`:>`, `?:`, `?'r`). [`lex_partial`] rather than `lex` so a file that stops
/// lexing part-way still contributes the tokens before the failure.
fn vocabulary() -> BTreeSet<String> {
    let v006 = corpus(&["lib-rustyfi/dist/packages", "layout-tests/corpus"]);
    let v01 = corpus(&["lib-rustyfi/dist-v01/packages"]);
    assert!(
        v006.len() > 100 && v01.len() > 20,
        "expected the bundled corpus, found {} + {} files — is the checkout complete?",
        v006.len(),
        v01.len()
    );
    let mut out = BTreeSet::new();
    for files in [&v006, &v01] {
        for f in files.iter() {
            let Ok(src) = std::fs::read_to_string(f) else {
                continue;
            };
            for version in VERSIONS {
                let (atoms, _err) = lex_partial(&src, version);
                for a in &atoms {
                    let s = a.span.start.byte.min(src.len());
                    let e = a.span.end.byte.min(src.len());
                    if let (true, Some(text)) = (s < e, src.get(s..e)) {
                        out.insert(text.to_string());
                    }
                }
            }
        }
    }
    out
}

const VERSIONS: [RustyfiVersion; 2] = [RustyfiVersion::V0_0, RustyfiVersion::V0_1];

// ---------------------------------------------------------------------------
// Placing a spelling where it can be adjacent to something
// ---------------------------------------------------------------------------

/// The shortest prefix that establishes each of the lexer's five modes.
///
/// Program mode first, so a spelling that is legal there is tested there — which
/// is both the commonest case and the only one the assertion covers.
const CONTEXTS: [(&str, &str); 5] = [
    ("program", ""),
    ("inline text", "{"),
    ("block text", "'<"),
    ("math", "${"),
    ("active", "\\c"),
];

/// A spelling, placed in a context, with everything the sweep needs precomputed.
struct Prev {
    /// The spelling itself.
    text: String,
    /// Which [`CONTEXTS`] entry it was placed in.
    ctx: usize,
    /// `context ++ text`, the left side of every concatenation.
    base: String,
    /// Byte offset of the join inside `base`.
    join: usize,
    /// Tokens ending at or before the join, per generation. What the
    /// concatenation must reproduce.
    prefix: [Vec<Token>; 2],
    /// Which generations actually place this spelling cleanly. A spelling can
    /// be one token under one lexer and not under the other (`:>`, `?'r`), and
    /// only the generations that cover it can say anything about a join.
    covered: [bool; 2],
    /// Is the lexer in *program* mode at the join, under either generation?
    program: bool,
}

/// The tokens of `s` that end at or before `join`, and whether one straddles it.
///
/// `Token::Eoi` is dropped: it is zero-width and sits at the end of the input,
/// so it would compare `base`'s end-of-input against the concatenation's
/// interior and answer "different" for every pair.
fn pre_join(s: &str, join: usize, version: RustyfiVersion) -> (Vec<Token>, bool, usize) {
    let (atoms, _err) = lex_partial(s, version);
    let mut toks = Vec::new();
    let mut last_end = 0usize;
    for a in &atoms {
        if a.slot == Token::Eoi {
            continue;
        }
        let (st, en) = (a.span.start.byte, a.span.end.byte);
        if en <= join {
            toks.push(a.slot.clone());
            last_end = en;
        } else if st < join {
            // One token covers bytes on both sides of the join: the fusion this
            // whole file exists to find.
            return (toks, true, last_end);
        } else {
            break;
        }
    }
    (toks, false, last_end)
}

/// Place `text` in the first context where its own bytes end at a token
/// boundary, under either generation.
///
/// "Ends at a token boundary" allows the tail of `text` to be *trivia* — a
/// spelling can end in whitespace (an inline-text `Break`, a `@require:` line's
/// own newline), and a boundary inside whitespace is still a boundary.
fn place(text: &str) -> Option<Prev> {
    for (ctx, (_, prefix)) in CONTEXTS.iter().enumerate() {
        let base = format!("{prefix}{text}");
        let join = base.len();
        let mut prefix_toks = [Vec::new(), Vec::new()];
        let mut covered = [false; 2];
        for (i, version) in VERSIONS.into_iter().enumerate() {
            let (toks, straddles, last_end) = pre_join(&base, join, version);
            covered[i] = !straddles
                && !toks.is_empty()
                && base[last_end..join].chars().all(char::is_whitespace);
            prefix_toks[i] = toks;
        }
        if !covered.iter().any(|c| *c) {
            continue;
        }
        let program = VERSIONS
            .into_iter()
            .any(|v| mode_at_join_is_program(&base, join, v));
        return Some(Prev {
            text: text.to_string(),
            ctx,
            base,
            join,
            prefix: prefix_toks,
            covered,
            program,
        });
    }
    None
}

/// Is the lexer in program mode at the join?
///
/// Measured, not inferred from the context: `\c` puts the lexer in an active
/// area, but `\c(` leaves it in program mode inside the parenthesis, and `{a}`
/// is back in program mode after the closing brace. The canary is `0x1F`, which
/// lexes to `IntConst(31)` in program mode and to something else in every other
/// mode — `Char("0x1F")` in inline text, `MathChar("0")` in math, a lex error in
/// a block-text or active area.
///
/// A space before the canary, because otherwise the canary answers the wrong
/// question: `foo` ++ `0x1F` is one identifier, so the probe would report "not
/// program mode" for every name in the corpus (measured: 3348 spellings instead
/// of 9739). Whitespace changes no mode in any of the five, and in an inline-text
/// area it produces a `Space`/`Break` token, which is not the canary either.
fn mode_at_join_is_program(base: &str, join: usize, version: RustyfiVersion) -> bool {
    let probe = format!("{base} 0x1F");
    let (atoms, _err) = lex_partial(&probe, version);
    atoms
        .iter()
        .find(|a| a.span.start.byte >= join && a.slot != Token::Eoi)
        .is_some_and(|a| a.slot == Token::IntConst(31))
}

/// Would writing `next` immediately after `prev` keep them two tokens?
///
/// The property stated in the module header: a boundary exactly at the join, and
/// the tokens before it unchanged. Checked under both generations; a pair is safe
/// only if it is safe under both, because the formatter runs under whichever one
/// the buffer says.
fn stays_two_tokens(prev: &Prev, next: &str) -> bool {
    let joined = format!("{}{}", prev.base, next);
    VERSIONS.into_iter().enumerate().all(|(i, version)| {
        if !prev.covered[i] {
            return true;
        }
        let (toks, straddles, _) = pre_join(&joined, prev.join, version);
        !straddles && toks == prev.prefix[i]
    })
}

// ---------------------------------------------------------------------------
// The character-class reduction, used only for the over-separation measurement
// ---------------------------------------------------------------------------

/// A representative of `c`'s *lexer-visible* character class.
///
/// Every ASCII punctuation and control character is its own class, because the
/// lexer matches most of them by name (`(`, `|`, `` ` ``, `#`, `@`, ...). Only
/// the letters and digits collapse, and they collapse along the seams the
/// lexer's own predicates cut:
///
/// - `is_hex` is `0-9 | A-F` — uppercase only — so `A-F` and `G-Z` are different
///   classes and lowercase letters are not hex at all;
/// - `0` and `x`/`X` head the hexadecimal prefix, so each is its own class;
/// - everything else in `a-z` behaves identically (`is_small`, `is_ident_char`,
///   `is_str_char`, `is_ascii_alphanumeric` all agree on it). Keyword-hood is
///   not a class distinction: the keyword table is consulted *after* the
///   identifier scan, so it cannot move a token boundary.
///
/// Non-ASCII splits exactly where `sep::is_word` does: `char::is_alphanumeric`
/// (Han, Kana, Hangul, non-ASCII digits) against everything else (`、`, `。`,
/// combining marks), both of which the lexer sees only through `is_str_char`.
fn class_rep(c: char) -> char {
    match c {
        '\t' => ' ',
        '0' | 'x' | 'X' | '_' => c,
        'A'..='F' => 'A',
        'G'..='W' | 'Y' | 'Z' => 'G',
        'a'..='w' | 'y' | 'z' => 'a',
        '1'..='9' => '1',
        _ if c.is_ascii() => c,
        _ if c.is_alphanumeric() => 'あ',
        _ => '、',
    }
}

/// `s` with every character replaced by its class representative and runs of one
/// class cut to four.
///
/// Four, not one: the longest fixed lookahead in the lexer is three characters
/// (`?->`), a backtick literal's `quote_len` is significant up to the longest
/// run the corpus uses, and `0x` + digits needs three. Anything longer than four
/// of one class cannot change a decision — every unbounded scan
/// (`scan_while(is_ident_char)`, the digit runs inside `length_lookahead`)
/// consumes character by character and so is decided by the first one.
fn canon(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = (' ', 0usize);
    for c in s.chars() {
        let r = class_rep(c);
        if r == run.0 {
            run.1 += 1;
        } else {
            run = (r, 1);
        }
        if run.1 <= 4 {
            out.push(r);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

/// Group spellings by their first character: [`sep::must_separate`] reads
/// nothing else about the right-hand side, so the whole group shares one answer
/// and the sweep can skip a group at a time.
fn by_first_char<'a>(vocab: &'a BTreeSet<String>) -> BTreeMap<char, Vec<&'a String>> {
    let mut out: BTreeMap<char, Vec<&'a String>> = BTreeMap::new();
    for s in vocab {
        if let Some(c) = s.chars().next() {
            out.entry(c).or_default().push(s);
        }
    }
    out
}

/// Split `items` into `n` interleaved shares, for `std::thread::scope`.
fn shares<T: Copy>(items: &[T], n: usize) -> Vec<Vec<T>> {
    let mut out = vec![Vec::new(); n];
    for (i, it) in items.iter().enumerate() {
        out[i % n].push(*it);
    }
    out
}

fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 16)
}

#[derive(Default, Debug)]
struct Counts {
    tested: u64,
    fusing: u64,
    /// One witness per boundary class (see [`boundary`]).
    witness: BTreeMap<(char, char), (String, String)>,
    /// Every boundary-character class the sweep actually reached, so a class the
    /// rule separates can be reported as an over-separation only if it was
    /// tested.
    classes: BTreeSet<(char, char)>,
}

impl Counts {
    fn merge(&mut self, other: Counts) {
        self.tested += other.tested;
        self.fusing += other.fusing;
        for (k, v) in other.witness {
            self.witness.entry(k).or_insert(v);
        }
        self.classes.extend(other.classes);
    }
}

/// The boundary of a pair, as the pair of *character classes*
/// [`sep::must_separate`] can actually distinguish (see [`class_rep`]).
///
/// Keyed by class rather than by raw character because that is the grain the rule
/// is written at: a hundred `@import:` lines behind one `(word, '@')` class are
/// one finding, and reporting 26 `('+', 'a'..'z')` rows separately buries the
/// others. The witness pair printed alongside carries the raw characters.
fn boundary(prev: &str, next: &str) -> (char, char) {
    (
        class_rep(prev.chars().next_back().expect("no empty spelling")),
        class_rep(next.chars().next().expect("no empty spelling")),
    )
}

/// Sweep every ordered pair `(p, n)` for which `want_separate == must_separate`.
///
/// `want_separate == false` is the soundness sweep: those are the pairs the rule
/// promises are safe, and a fusing one is a bug. `want_separate == true` is the
/// quality sweep: a *non*-fusing pair there is an over-separation.
fn sweep(prevs: &[&Prev], nexts: &BTreeMap<char, Vec<&String>>, want_separate: bool) -> Counts {
    let n = threads();
    let mut total = Counts::default();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for share in shares(prevs, n) {
            handles.push(scope.spawn(move || {
                let mut c = Counts::default();
                for p in share {
                    for (first, group) in nexts {
                        // One `must_separate` call decides the whole group.
                        if sep::must_separate(&p.text, &first.to_string()) != want_separate {
                            continue;
                        }
                        // The class is a property of the group, not of the pair.
                        c.classes.insert(boundary(&p.text, &first.to_string()));
                        for next in group {
                            c.tested += 1;
                            if !stays_two_tokens(p, next) {
                                c.fusing += 1;
                                c.witness
                                    .entry(boundary(&p.text, next))
                                    .or_insert_with(|| (p.text.clone(), (*next).clone()));
                            }
                        }
                    }
                }
                c
            }));
        }
        for h in handles {
            total.merge(h.join().expect("no sweep thread panics"));
        }
    });
    total
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// How much of the vocabulary a sweep runs over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    /// Every distinct spelling the corpus contains, verbatim. Exhaustive, and
    /// slow enough (~40 s) to be `#[ignore]`d.
    Real,
    /// The character-class reduction (see [`canon`]). `canon` already leaves every
    /// operator and delimiter spelling *unchanged* — each ASCII punctuation
    /// character is its own class — so what it actually collapses is the
    /// identifiers, numbers, literals and prose; one- and two-character spellings
    /// are kept verbatim on top of that, belt and braces.
    Reduced,
}

/// Everything one sweep measured. Printed in full *before* anything is asserted,
/// so a failing run still reports its numbers instead of dying at the first one.
struct Report {
    scope: Scope,
    vocab: usize,
    swept: usize,
    kept_verbatim: usize,
    placed: usize,
    unplaceable: Vec<String>,
    per_ctx: [usize; CONTEXTS.len()],
    program: usize,
    text_mode: usize,
    sound: Counts,
    over: Counts,
    /// Which real corpus spelling each swept spelling stands for.
    example: BTreeMap<String, String>,
}

fn run_sweep(scope: Scope) -> Report {
    let vocab = vocabulary();
    let (swept, example, kept_verbatim) = match scope {
        Scope::Real => (
            vocab.clone(),
            vocab.iter().map(|s| (s.clone(), s.clone())).collect(),
            vocab.len(),
        ),
        Scope::Reduced => reduced_vocabulary(&vocab),
    };

    let mut placed: Vec<Prev> = Vec::new();
    let mut unplaceable: Vec<String> = Vec::new();
    for s in &swept {
        match place(s) {
            Some(p) => placed.push(p),
            None => unplaceable.push(s.clone()),
        }
    }
    let mut per_ctx = [0usize; CONTEXTS.len()];
    for p in &placed {
        per_ctx[p.ctx] += 1;
    }
    let program: Vec<&Prev> = placed.iter().filter(|p| p.program).collect();
    let text_mode = placed.len() - program.len();
    let nexts = by_first_char(&swept);

    Report {
        scope,
        vocab: vocab.len(),
        swept: swept.len(),
        kept_verbatim,
        placed: placed.len(),
        unplaceable,
        per_ctx,
        program: program.len(),
        text_mode,
        // The soundness sweep: the pairs the rule promises are safe.
        sound: sweep(&program, &nexts, false),
        // The quality sweep: the pairs it separates, of which the ones that
        // would not have fused are the over-separations.
        over: sweep(&program, &nexts, true),
        example,
    }
}

impl Report {
    fn print(&self) {
        println!("---- sep sweep, scope {:?} ----", self.scope);
        println!("distinct token spellings in the corpus: {}", self.vocab);
        println!(
            "swept: {} spellings ({} of them real corpus spellings, kept verbatim)",
            self.swept, self.kept_verbatim
        );
        println!(
            "placed: {} ; unplaceable in any of the five modes: {}",
            self.placed,
            self.unplaceable.len()
        );
        for (i, (name, _)) in CONTEXTS.iter().enumerate() {
            println!("  context {name:>12}: {} spellings", self.per_ctx[i]);
        }
        for s in self.unplaceable.iter().take(6) {
            println!("  unplaceable: {s:?}");
        }
        println!(
            "program mode at the join: {} (asserted) ; a text/math/active area at \
             the join: {} (measured only — see the domain test)",
            self.program, self.text_mode
        );
        println!(
            "soundness sweep: {} pairs tested (must_separate said false), {} fuse",
            self.sound.tested, self.sound.fusing
        );
        for ((a, b), (p, n)) in &self.sound.witness {
            println!("  UNSOUND class ({a:?}, {b:?}): {:?} ++ {:?}", self.real(p), self.real(n));
        }
        println!(
            "over-separation: {} pairs where must_separate says true, of which {} \
             really fuse -> {} over-separations ({:.1}%), in {} boundary classes",
            self.over.tested,
            self.over.fusing,
            self.over.tested - self.over.fusing,
            100.0 * (self.over.tested - self.over.fusing) as f64 / self.over.tested.max(1) as f64,
            self.over_separating_classes().len(),
        );
        let classes = self.over_separating_classes();
        println!(
            "  over-separated classes (first 24 of {}): {:?}",
            classes.len(),
            &classes[..classes.len().min(24)]
        );
    }

    /// A real corpus spelling the swept one stands for, for a legible witness.
    fn real(&self, swept: &str) -> String {
        self.example.get(swept).cloned().unwrap_or_else(|| swept.to_string())
    }

    /// Boundary classes the rule separates although nothing in them ever fuses.
    fn over_separating_classes(&self) -> Vec<(char, char)> {
        let fusing: BTreeSet<(char, char)> = self.over.witness.keys().copied().collect();
        let mut out: Vec<(char, char)> = self
            .over
            .classes
            .iter()
            .filter(|c| !fusing.contains(c))
            .copied()
            .collect();
        out.sort_unstable();
        out
    }

    fn assert_sound(&self) {
        assert!(
            self.vocab > 5_000,
            "the vocabulary shrank to {} — the corpus sweep has gone quiet",
            self.vocab
        );
        // A spelling no context accepts as one token tells this file nothing, so
        // it is reported rather than assumed away — but if most of the input
        // ended up there, the sweep is a shadow of its corpus.
        assert!(
            self.unplaceable.len() * 20 < self.swept,
            "{} of {} swept spellings could not be placed in any of the five \
             modes — the sweep is mostly skipping its input",
            self.unplaceable.len(),
            self.swept
        );
        assert!(
            self.program * 3 > self.placed,
            "only {} of {} placed spellings reach a program-mode join",
            self.program,
            self.placed
        );
        assert!(
            self.sound.tested > 500_000,
            "the soundness sweep tested only {} pairs — the rule now answers \
             `true` for almost everything, so this test has stopped checking it",
            self.sound.tested
        );
        // Not vacuous in the other direction either: the sweep must actually be
        // finding fusions among the pairs the rule DOES separate, or the whole
        // separation table is measuring nothing.
        assert!(
            self.over.fusing > 1_000,
            "only {} of the separated pairs really fuse — the fusion detector \
             itself has stopped working",
            self.over.fusing
        );
        assert_eq!(
            self.sound.fusing,
            0,
            "{} pairs in {} boundary-character classes fuse although \
             must_separate says no separator is needed: {:?}. A false `false` \
             corrupts a document.",
            self.sound.fusing,
            self.sound.witness.len(),
            self.sound.witness.keys().collect::<Vec<_>>(),
        );
    }
}

/// The shipped sweep: exhaustive over the class-reduced vocabulary.
#[test]
fn must_separate_is_sound_over_every_pair_of_corpus_token_spellings() {
    let report = run_sweep(Scope::Reduced);
    report.print();
    report.assert_sound();
}

/// The same sweep over the raw corpus vocabulary — 11770 spellings, ~66 M pairs,
/// ~40 s. `#[ignore]`d for the running time, not for any doubt about it: it is
/// what proves the class reduction above loses nothing, and it is the run to
/// make after touching `sep.rs`'s rule.
///
///     cargo test -p rustyfi-lsp --test format_cst_sep -- --ignored --nocapture
#[test]
#[ignore]
fn must_separate_is_sound_over_the_raw_corpus_vocabulary() {
    let report = run_sweep(Scope::Real);
    report.print();
    report.assert_sound();
}

/// The class reduction, plus its own casualties.
///
/// Returns the swept vocabulary, a real corpus spelling for each entry, and how
/// many entries are real spellings rather than class representatives.
///
/// A spelling whose canonical form does not *behave* like it — placed in a
/// different context, or not placeable at all — is kept verbatim, so the
/// reduction cannot quietly drop a shape. `@require: foo` is the standing
/// example: the class map turns its header keyword into `aaaa`, which is not a
/// header at all.
fn reduced_vocabulary(
    vocab: &BTreeSet<String>,
) -> (BTreeSet<String>, BTreeMap<String, String>, usize) {
    let mut out = BTreeSet::new();
    let mut example: BTreeMap<String, String> = BTreeMap::new();
    let mut verbatim = 0usize;
    let profile = |t: &str| place(t).map(|p| (p.ctx, p.program));
    for s in vocab {
        let c = canon(s);
        // `canon` is already the identity on every operator and delimiter
        // spelling, since each ASCII punctuation character is its own class.
        // One- and two-character spellings are kept verbatim anyway: it costs
        // ~200 entries and removes the need to trust that reasoning.
        let keep = s.chars().count() <= 2 || profile(&c) != profile(s);
        let entry = match keep {
            true => {
                verbatim += 1;
                s.clone()
            }
            false => c,
        };
        example.entry(entry.clone()).or_insert_with(|| s.clone());
        out.insert(entry);
    }
    (out, example, verbatim)
}

/// Why [`sep::must_separate`]'s domain is program-area adjacency, as a test.
///
/// Inside an inline-text area almost every character is `is_str_char`, so two
/// adjacent copied ranges fuse into one `Char` run whatever their classes are —
/// `prose` ++ `!more` is one token, and no character-class rule could say
/// otherwise without also separating `f` from `(` in program mode. The formatter
/// does not need one to: a text area is copied through as a single
/// `Doc::Verbatim` whose interior is never re-spaced, and inserting a space in
/// SATySFi prose would change the typeset output anyway.
///
/// This is asserted rather than remarked so that the precondition cannot be
/// forgotten while `sep.rs` is being tightened: if someone ever makes the rule
/// sound here too, this test fails and asks them to explain what happened to
/// `f(x)`.
#[test]
fn the_table_is_unsound_inside_a_text_area_which_is_why_its_domain_is_program_mode() {
    let prose = place("prose").expect("placeable");
    assert_eq!(CONTEXTS[prose.ctx].0, "program", "`prose` is a program-mode name");

    // Forced into an inline-text area, the same two ranges fuse.
    let in_text = Prev {
        text: "prose".to_string(),
        ctx: 1,
        base: "{prose".to_string(),
        join: 6,
        prefix: [
            pre_join("{prose", 6, RustyfiVersion::V0_0).0,
            pre_join("{prose", 6, RustyfiVersion::V0_1).0,
        ],
        covered: [true, true],
        program: false,
    };
    assert!(!sep::must_separate("prose", "!more"));
    assert!(
        !stays_two_tokens(&in_text, "!more"),
        "if this fuses no longer, the inline-text lexer has changed and this \
         test's premise needs re-reading"
    );
    // And in program mode, where the table IS consulted, the same pair is safe.
    assert!(stays_two_tokens(&prose, "!more"));

    // The CJK question, answered: `is_word` is `char::is_alphanumeric`, which is
    // true for Han and Kana, so two adjacent CJK ranges WOULD be separated — and
    // a space between two runs of Japanese prose is not cosmetic, it changes the
    // typeset line. It cannot arise: a CJK spelling is not a program-mode token
    // at all, so it never reaches a program-mode join.
    assert!(sep::must_separate("日本語", "とはいえ"));
    let cjk = place("日本語").expect("placeable in an inline-text area");
    assert_ne!(CONTEXTS[cjk.ctx].0, "program");
    assert!(!cjk.program, "a CJK run never sits at a program-mode join");
}

/// The reduction's own arithmetic, so a broken `canon` is not silently a
/// *smaller* sweep.
#[test]
fn the_class_reduction_collapses_what_it_claims_to() {
    assert_eq!(canon("get-font-size"), "aaa-aaaa-aaaa");
    assert_eq!(canon("0x1F"), "0x1A");
    assert_eq!(canon("Mod.foo"), "Gaa.aaa"); // `M` is not A-F, `o`/`d` are ordinary lowercase
    assert_eq!(canon("12345678"), "1111");
    assert_eq!(canon("````"), "````");
    assert_eq!(canon("`````"), "````");
    assert_eq!(canon("日本語です"), "ああああ");
    assert_eq!(canon("、。"), "、、");
    // Whitespace: tab folds into space, the two line terminators do not fold
    // into each other (a header token's CRLF is one break and both halves are
    // inside its span).
    assert_eq!(canon(" \t "), "   ");
    assert_eq!(canon("\r\n"), "\r\n");
}
