//! The CST-based formatter.
//!
//! # What this is
//!
//! The layout IR ([`doc`]), the renderer ([`render`]), the trivia scan
//! ([`trivia`]), the separation table ([`sep`]) and the verifier, wired to a
//! builder per grammar ([`build006`], [`build01`]) that decides indentation,
//! spacing and **line structure**. It reformats: it collapses and inserts
//! whitespace, joins short lines, wraps long ones, and re-wraps prose.
//!
//! It began as slice 0, a **provable no-op** — every token and every gap
//! emitted verbatim, byte-identical output on any file that lexes — and that
//! ordering was deliberate: the safety net is worth having *before* the first
//! aesthetic decision. That no-op property is gone by construction, and what
//! replaced it is the pair of sweep properties in `format_cst_slice1.rs`:
//! the output re-lexes to the **same token stream** as the input, and each
//! token's own source bytes are copied through unchanged. Those still admit
//! no judgement; the layout on top of them is all judgement.
//!
//! Read that file's module doc before changing a break rule. Five of six
//! break-rule mutations are **invisible to both corpus sweeps** — a wrong
//! break changes no token, moves no non-whitespace byte, and is a fixpoint —
//! so the hand-written layout fixtures, not the sweeps, are the gate for
//! anything about *where* a line breaks.
//!
//! [`crate::format`] is the older lex-based formatter and is still what the
//! **server** calls; the playground calls this module, across
//! `rustyfi_format`. It also serves as this module's own **tier 2** (below).
//!
//! # The tier list
//!
//! `engine.md` section 8, wired: a buffer that does not **lex** declines (tier
//! 0), one that lexes and **parses** is laid out (tier 1), and one that lexes
//! but does not parse falls through to [`crate::format`] (tier 2). So this
//! module does not replace the lex-based formatter — it *contains* it, and
//! `format.rs` must not be deleted.
//!
//! Tier 2 used to be the identity builder, and that was the silent-failure
//! defect rather than a conservative stand-in: the identity's output is
//! byte-for-byte what an ALREADY FORMATTED file looks like, so `rustyfi fmt
//! --check` could not tell them apart and exited `0` over a file the formatter
//! had never touched. [`CstOutcome`] is the fix — the outcome, not the bytes,
//! is what says which tier ran.
//!
//! # Why a parse at all
//!
//! The lex-based formatter can only rewrite the bytes strictly between two
//! program-area token spans, because a token stream does not say how the program
//! nests. That is why it never re-indents (`format.rs:88`), and the reason is
//! measured rather than assumed: a bracket counter, the only nesting a token
//! stream affords, gets `itemize.satyh:48-52` wrong, where an argument line is
//! indented one step past its function with no bracket opened in between
//! (`format.rs:94-110`).
//!
//! # Intra-line spacing: one rule, four areas, one exception
//!
//! The spacing rules used to be a list of SHAPES — `=`, `->`, `:`, `|`, a
//! binop, `;`, `,`, a bracket, an argument boundary — each taught at the CST
//! site that knew it, with every gap no shape claimed copied verbatim. That
//! arrangement produced the same bug report five times, because the failure
//! mode of a shape list is SILENCE: a gap nobody named is not mis-formatted,
//! it is untouched, and untouched is indistinguishable from deliberate.
//!
//! The default is inverted now. **Every gap that holds only horizontal
//! whitespace is rewritten**, and the exceptions are a named, counted list on
//! [`build006::Spacing`]. A shape nobody thought of gets one space rather than
//! nothing, so the failure mode flips from "did nothing" to "did the ordinary
//! thing".
//!
//! Which gaps that reaches is a question about AREAS, and the answer is one
//! property rather than five decisions:
//!
//! ```text
//!   area                          whitespace                   treatment
//!   program                       no token                     rewritten
//!   active (`\cmd` args, `;`)     no token (lexer.rs:1241)     rewritten
//!   math `${ }`                   no token (lexer.rs:1338)     rewritten
//!   block text `'< >`             no token (lexer.rs:1029)     rewritten
//!   inline text `{ }`             ONE token per run, identity
//!                                 fixed by its first character  frozen
//! ```
//!
//! Four of the five carry no information for the typesetter — measured, not
//! assumed: `${x   +   y}`, `${x + y}` and `${x+y}` compile to byte-identical
//! PDFs, against a `${x - y}` control that differs — so they get one code
//! path. Inline text is the single exception, and it has its own machinery
//! ([`inline`]'s measured re-wrap predicate) rather than an exception inside
//! this one. That is the fact the last five bug reports were circling.
//!
//! Two guards are unchanged by the inversion, and they are what keeps a
//! universal request safe: a gap holding a **line break** or a **`%` comment**
//! is dropped before any request is consulted, so no rule can join two lines
//! or eat a comment.
//!
//! # Where trivia comes from
//!
//! Not from the tree — the lexer discards it before the parser sees it. But the
//! token spans **tile** the source: `Lexer::emit` (`lexer.rs:281-286`) stamps
//! every span from a cursor that only advances, so
//!
//! ```text
//! src == leading_gap ++ for each atom: (its own bytes ++ the gap after it)
//! ```
//!
//! and every gap holds nothing but spaces, tabs, line breaks and `%` comments.
//! That is what [`trivia`] reads, and it is pinned independently by
//! `rustyfi-syntax`'s `tests/atoms_roundtrip.rs`, which asserts over 209 corpus
//! files that unparsing a parsed file replays the lexed atoms with **identical
//! spans** — the property this module's addressing depends on and the older
//! `tests/roundtrip.rs` does not check.

pub(crate) mod build006;
pub(crate) mod build01;
pub(crate) mod comment;
pub(crate) mod doc;
pub(crate) mod inline;
pub(crate) mod render;
pub(crate) mod sep;
pub(crate) mod trivia;

use rustyfi_syntax::parse_error::locate;
use rustyfi_syntax::token::Atom;
use rustyfi_syntax::{lex_with_version, ParseFailureKind, ParseFileError, RustyfiVersion, Token};
use syan::parse::unparse::{Emitter, Unparse};
use syan::parse::Parse;

use doc::Doc;

/// Slice 0's options. Deliberately the fields the design's stable set names,
/// no more: the option surface is decided in `docs/plans/formatter-cst/config.md`
/// and admitting a key is a seven-clause argument, not a convenience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CstOptions {
    pub max_width: usize,
    pub tab_spaces: usize,
    pub max_blank_lines: usize,
    /// Reflow an own-line `%` comment that [`comment::is_prose`] accepts and
    /// whose line is wider than [`CstOptions::max_width`].
    ///
    /// **Default `true`.** It was `false`, on the census below; the user
    /// chose `true` with that census in front of them, and the number is
    /// recorded here because it says what the option costs rather than what
    /// it should be set to.
    ///
    /// Over the 209 files of `lib-rustyfi/dist/packages` +
    /// `layout-tests/corpus` there are 2062 `%` comments in program-area gaps
    /// that this rule can reach at all; 30 sit on a line over the budget, 23
    /// would come under it if reflowed, and **3** survive
    /// [`comment::is_prose`] and are own-line. (The doc comment used to quote
    /// 2474, which counted every `%` in the corpus rather than the population
    /// the rule reaches; the smaller figure is the honest denominator and the
    /// numerator is unchanged.)
    ///
    /// So on this corpus the option is nearly inert, and what it is really
    /// buying is a codebase whose comments are English doc prose at 120
    /// columns — the corpus is not every codebase. The risk it opens is
    /// rewriting somebody's commented-out CODE into something that no longer
    /// uncomments cleanly, and what makes turning it on survivable is that
    /// the classifier's bias runs the safe way: measured 22% false *reject*
    /// against 1.7% false *accept*. See [`comment`]'s module documentation
    /// for both rates and the samples they came from.
    pub wrap_comments: bool,
    /// Re-wrap the **text inside `{ … }`** to [`CstOptions::max_width`],
    /// gap by gap.
    ///
    /// A gap that [`inline`]'s measured predicate clears may be broken or
    /// joined to fill the line; one it refuses is frozen exactly as the author
    /// wrote it, neither joined nor split. Continuation lines take the area's
    /// own indentation, which slice 4 already computes.
    ///
    /// # What it costs, measured — and the measurement is MIXED
    ///
    /// Both corpora, formatted with this key on against the same files with it
    /// off:
    ///
    /// ```text
    ///                       files changed        lines
    ///   0.0.6 (162 files)      17            -405 / +437
    ///   0.1    (47 files)       5              -7 /  +14
    /// ```
    ///
    /// **The upside is large and the downside is real, and they are not the
    /// same files.** A 230-column paragraph — the report this feature exists
    /// for — becomes readable at any budget. But `layout-tests/corpus`'s
    /// Japanese manuals are written ONE SENTENCE PER LINE, and this rule
    /// replaces that with column filling:
    ///
    /// ```text
    ///   -  もう少し自由度高くラベルを設定したい場合もあるでしょう。
    ///   -  `text-label` 関数を用いると、任意の…ことができます。
    ///   +  もう少し自由度高くラベルを設定したい場合もあるでしょう。 `text-label`
    ///   +  関数を用いると、任意の…ことができます。
    /// ```
    ///
    /// Nothing there is unsafe — every gap it moved is one the typesetter
    /// provably cannot see, and `crates/rustyfi/tests/ws_inline_rewrap.rs`
    /// compiles seven corpus documents before and after and gets byte-identical
    /// PDFs. It is a TASTE regression: the author's breaks carried sentence
    /// structure, the formatter's carry column width, and because the predicate
    /// freezes the CJK/CJK gaps the fill can only break at the few places a
    /// command or a literal happens to sit — so the result is lopsided as well
    /// as arbitrary.
    ///
    /// So the number does not settle the default on its own, and the key is
    /// how a project says which it wants. A codebase of Latin prose gets the
    /// feature it asked for; a project whose manuals are laid out by hand sets
    /// this to `false` and keeps slice 4's re-indentation, which is the whole
    /// of what it had before.
    ///
    /// # And why it is a flag rather than a rule
    ///
    /// Because "the same document, re-wrapped" is a large diff over prose
    /// somebody laid out by hand, and a project that wants its manuals left
    /// alone should be able to say so in one key rather than by turning the
    /// formatter off.
    pub wrap_inline_text: bool,
}

impl Default for CstOptions {
    fn default() -> Self {
        CstOptions {
            // Satisfied by >=98.7% of existing corpus lines in every group,
            // measured in display columns. A budget the corpus routinely
            // violated would make the first run a catastrophic diff.
            max_width: 100,
            // 83.3% of 9081 indentation increments across 209 corpus files are
            // 2 columns; 14.2% are 4. Not rustfmt's 4.
            tab_spaces: 2,
            // The corpus uses a two-blank-line gap as a section break and only
            // 12 lines in 24111 exceed it (`format.rs:163-172`).
            max_blank_lines: 2,
            // Three comments in 2062 on this corpus, and a real reflow on a
            // codebase whose comments are prose. The user's call, made with
            // the census in front of them — see the field's own
            // documentation.
            wrap_comments: true,
            // Reaches 80.7% of the corpus's inline gaps and all 246 of its
            // multi-line inline areas; freezes the 429 the predicate refuses.
            // 17 of 162 files change, and the field's own documentation says
            // plainly which of those changes are an improvement and which are
            // not — this default is a judgement call over a mixed measurement,
            // not a number.
            wrap_inline_text: true,
        }
    }
}

/// Collect a node's atoms in source order.
///
/// The bridge from the typed CST to byte offsets, and the reason this design
/// needs no change in `rustyfi-syntax`. CST nodes derive `Parse`/`Unparse` but
/// deliberately **not** syan's `Spanned` — `walk006.rs:3-11` explains that
/// adding a third derive to the `#[recurse]` grammar module is not affordable
/// (`docs/syan-api-recurse-performance.md` records >16 min codegen and >10 GB
/// rustc RSS on a smaller grammar). But `Unparse` already replays a node's atoms
/// in source order, which `symbols.rs:282-308` already exploits to get a node's
/// *extent* by uniting the spans it emits. Collecting them instead of uniting
/// them gives the token list, for either grammar, for free.
pub(crate) fn atoms_of<T: Unparse<Atom> + ?Sized>(node: &T) -> Vec<Atom> {
    struct Collect(Vec<Atom>);
    impl Emitter<Atom> for Collect {
        type Error = std::convert::Infallible;
        fn write_one(&mut self, atom: Atom) -> Result<(), Self::Error> {
            self.0.push(atom);
            Ok(())
        }
        fn write_sep(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    let mut sink = Collect(Vec::new());
    // `Collect` cannot fail, so this discards an `Infallible`.
    let _ = node.unparse(&mut sink);
    sink.0
}

/// Why [`format_cst_outcome`] produced no text at all.
///
/// Two genuinely different failures, and conflating them was half of the
/// silent-failure bug this type exists to end: one is a claim about the
/// **input**, the other about **this code**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclineReason {
    /// Tier 0. The buffer does not lex, so there is no area map and no token
    /// stream to re-emit — the mode stack is what failed. `format.rs:127-134`'s
    /// reflex, kept verbatim.
    DoesNotLex,
    /// The printer produced text that lexes to a *different* token stream, so
    /// the always-on verifier rejected it.
    ///
    /// `engine.md` section 8's tier table says a verify failure "demotes to
    /// tier 2". It does not here, and that is deliberate: a verifier failure is
    /// an unknown bug in this module, and quietly serving the whitespace-only
    /// formatter's output instead would hide it behind a plausible answer. The
    /// module's own rule — "every unknown bug surfaces as *the formatter did
    /// nothing*" — is worth more than the small convenience, and the reason a
    /// parse failure could not stay silent is precisely that it is **not** a
    /// bug in this module and so had no such signal.
    VerifierRejected,
}

/// What one call to the CST formatter actually did.
///
/// The type exists because `Option<String>` could not tell "I formatted this"
/// from "I could not parse it, so I handed the bytes back". Both were
/// `Some(text)`, so `rustyfi fmt --check` reported a file the formatter never
/// touched as clean and exited `0` — CI green over a formatter that did
/// nothing. Every arm below is a state a caller has a different answer for.
/// Not `PartialEq`: [`ParseFailureKind`] is, but [`ParseFileError`] is not
/// (it carries a rendered message), and an equality that quietly ignored the
/// failure would be the same conflation this type exists to end. Match on the
/// arms.
#[derive(Debug, Clone)]
pub enum CstOutcome {
    /// Tier 1: it lexed, it parsed, the builder laid it out, and the result
    /// differs from the input.
    Formatted(String),
    /// Tier 1, and the input was already exactly what the formatter would
    /// write. The text is `source` byte for byte.
    AlreadyFormatted(String),
    /// Tier 2: it lexed but did not **parse**, so the text is
    /// [`crate::format`]'s — whitespace-only, and strictly more than the
    /// identity this used to return.
    ///
    /// `changed` says whether tier 2 nonetheless improved the buffer, so a
    /// caller can report "unparsed" and "needs reformatting" independently.
    FellBack {
        text: String,
        changed: bool,
        error: ParseFileError,
    },
    /// No text at all. See [`DeclineReason`].
    Declined(DeclineReason),
}

impl CstOutcome {
    /// The bytes to write, or `None` for a decline.
    ///
    /// The whole of what [`format_cst`] can say, which is why that function is
    /// two lines and this type is the real answer.
    pub fn text(&self) -> Option<&str> {
        match self {
            CstOutcome::Formatted(t)
            | CstOutcome::AlreadyFormatted(t)
            | CstOutcome::FellBack { text: t, .. } => Some(t),
            CstOutcome::Declined(_) => None,
        }
    }

    /// Did the parse succeed? `false` for tier 2 **and** for a decline, since
    /// neither established that the file is syntactically whole.
    pub fn parsed(&self) -> bool {
        matches!(
            self,
            CstOutcome::Formatted(_) | CstOutcome::AlreadyFormatted(_)
        )
    }

    /// The parse failure, for a tier-2 outcome only.
    pub fn parse_error(&self) -> Option<&ParseFileError> {
        match self {
            CstOutcome::FellBack { error, .. } => Some(error),
            _ => None,
        }
    }

    /// Do the returned bytes differ from the input?
    pub fn changed(&self) -> bool {
        match self {
            CstOutcome::Formatted(_) => true,
            CstOutcome::AlreadyFormatted(_) => false,
            CstOutcome::FellBack { changed, .. } => *changed,
            CstOutcome::Declined(_) => false,
        }
    }

    /// How good an answer this is, for a caller choosing between two
    /// generations' attempts. Higher wins; ties keep the first.
    ///
    /// `engine.md` section 8's version-selection rule — "treat a tier-2 outcome
    /// under one generation as a reason to try the other before settling" — is
    /// exactly this ordering, and it lives here rather than in `rustyfi fmt` so
    /// that the CLI, the server and the playground cannot disagree about it.
    pub fn rank(&self) -> u8 {
        match self {
            CstOutcome::Formatted(_) | CstOutcome::AlreadyFormatted(_) => 2,
            CstOutcome::FellBack { .. } => 1,
            CstOutcome::Declined(_) => 0,
        }
    }
}

/// Format `source` under `version`, saying which of the four things happened.
///
/// This is `engine.md` section 8's tier list, wired:
///
/// | tier | input | behaviour |
/// |---|---|---|
/// | 0 | does not lex | [`CstOutcome::Declined`] |
/// | 1 | lexes and parses | full format |
/// | 2 | lexes, does not parse | [`crate::format`]'s whitespace-only output |
///
/// Tier 2 used to be the *identity* builder plus the renderer's final newline,
/// which is strictly worse than the formatter it was supposed to fall back to:
/// `crate::format` still trims trailing whitespace, expands tabs, caps blank
/// runs and drops leading blank lines on a buffer that does not parse, and its
/// whole design point is being safe on exactly such a buffer. Worse, the
/// identity output was indistinguishable from "already formatted", so one
/// unparseable line silenced the formatter for a whole file **and reported
/// success**.
///
/// # `GaveUp` is not "invalid"
///
/// [`ParseFailureKind::GaveUp`] means the backtracking budget ran out and says
/// explicitly that the file may well be valid. Two things follow, and both are
/// implemented rather than assumed:
///
/// - the parse here spends the **compiler's** scaling `Budget::for_atoms`
///   (`AtomStream::new`), never `crate::budget::BUDGET`'s flat editor
///   allowance — so a large valid file that gives up in the diagnostics pane
///   still gets a full format. That IS the retry `engine.md` section 8 demands;
///   there is no smaller budget to retry *from*.
/// - the kind is carried out in [`CstOutcome::FellBack`]'s `error`, so a caller
///   reports "gave up" as a give-up rather than as a syntax error. Demoting a
///   give-up to a plain "this file is broken" would be the same silent
///   demotion, one level up.
pub fn format_cst_outcome(
    source: &str,
    version: RustyfiVersion,
    opts: &CstOptions,
) -> CstOutcome {
    if source.is_empty() {
        return CstOutcome::AlreadyFormatted(String::new());
    }
    let Ok(atoms) = lex_with_version(source, version) else {
        return CstOutcome::Declined(DeclineReason::DoesNotLex);
    };
    let doc = match slice1_doc(source, &atoms, version, opts) {
        Ok(doc) => doc,
        // Tier 2. The builder is not consulted at all: `crate::format` reads
        // the token stream, which is the only thing this buffer has.
        Err(error) => return fall_back(source, version, opts, error),
    };
    let rendered = render::render(
        &doc,
        &render::Options {
            max_width: opts.max_width,
            indent: opts.tab_spaces,
            newline: dominant_newline(source),
            max_blank_lines: opts.max_blank_lines,
        },
    );
    // The verifier. Always on, not behind `debug_assertions`: the cost is one
    // lex of the output, and shipping a printer whose unknown bugs corrupt
    // documents silently is not a trade a tool that rewrites other people's
    // files gets to make. A failure DECLINES rather than returning the text, so
    // every unknown bug surfaces as "the formatter did nothing".
    if !same_tokens(source, &rendered, version) {
        return CstOutcome::Declined(DeclineReason::VerifierRejected);
    }
    match rendered == source {
        true => CstOutcome::AlreadyFormatted(rendered),
        false => CstOutcome::Formatted(rendered),
    }
}

/// Tier 2: hand the buffer to the whitespace-only formatter.
///
/// The options are derived from [`CstOptions`] rather than defaulted, so that
/// a tier-2 file and a tier-1 file in the same run get the same blank-line cap
/// and the same tab width. The three LSP booleans are fixed `true` because they
/// are the normalisations `rustyfi fmt` performs unconditionally on every
/// tier-1 file — a tier-2 file must not silently opt out of them.
///
/// The verifier runs here too. `crate::format` is corpus-tested for token
/// identity and never writes a byte inside a token span, so this should be
/// unreachable — which is the reason to check rather than the reason not to:
/// tier 2's output is now shipped to users' files and the property is asserted,
/// not assumed.
fn fall_back(
    source: &str,
    version: RustyfiVersion,
    opts: &CstOptions,
    error: ParseFileError,
) -> CstOutcome {
    let fopts = crate::format::FormatOptions {
        tab_size: opts.tab_spaces,
        insert_spaces: true,
        trim_trailing_whitespace: true,
        insert_final_newline: true,
        trim_final_newlines: true,
        max_blank_lines: opts.max_blank_lines,
    };
    let Some(text) = crate::format::format(source, version, &fopts) else {
        // `format` declines only for a buffer that does not lex (which this one
        // did) or for its own drifted-area backstop. Either way there are no
        // bytes, and the decline reason is the honest one.
        return CstOutcome::Declined(DeclineReason::VerifierRejected);
    };
    if !same_tokens(source, &text, version) {
        return CstOutcome::Declined(DeclineReason::VerifierRejected);
    }
    let changed = text != source;
    CstOutcome::FellBack {
        text,
        changed,
        error,
    }
}

/// Format `source` under `version`.
///
/// The thin wrapper over [`format_cst_outcome`], kept so the corpus sweeps and
/// the playground read as they did. `None` still means **declined** — and note
/// what it does NOT mean: a buffer that lexed but did not parse now comes back
/// as `Some`, holding `crate::format`'s output. A caller that needs to tell
/// that from a real format — `rustyfi fmt --check` does, and exits `0` wrongly
/// without it — must call [`format_cst_outcome`].
pub fn format_cst(source: &str, version: RustyfiVersion, opts: &CstOptions) -> Option<String> {
    match format_cst_outcome(source, version, opts) {
        CstOutcome::Declined(_) => None,
        out => out.text().map(str::to_string),
    }
}

/// The column budget an own-line comment is reflowed to, or `None` for "never
/// reflow a comment" — which is the default.
///
/// One function rather than two `if`s at the two call sites, so that turning
/// the feature off is one place and both grammars provably agree.
fn wrap_budget(opts: &CstOptions) -> Option<usize> {
    opts.wrap_comments.then_some(opts.max_width)
}

/// Slice 1's builder, or the parse failure that sends this buffer to tier 2.
///
/// The error is a real [`ParseFileError`] — position, message and
/// [`ParseFailureKind`] — rather than a bare `None`, because tier 2's whole
/// point is that the user is told *why* their file was only whitespace-tidied.
/// It is produced by `rustyfi_syntax::parse_error::locate`, the same function
/// `cst::parse_file` uses, so the formatter's message and the compiler's are
/// the same message.
///
/// The parse spends the **compiler's** scaling budget
/// (`Budget::for_atoms`, via `AtomStream::new`) rather than
/// `crate::budget::BUDGET`. `engine.md` section 8 is explicit about why:
/// `ParseFailureKind::GaveUp` means the flat editor budget ran out and says
/// "the file may well be valid", so a large but perfectly valid file can give
/// up in the editor and parse fine in the compiler. Formatting is
/// user-initiated and once per save; it can afford the parse that diagnostics
/// cannot. That is the retry, spent up front: there is no second, larger budget
/// to escalate to short of `Budget::unlimited`, which would trade a
/// misclassified file for a hung editor.
///
/// A builder that returns `None` — a shape it has no layout rule for — is also
/// routed to tier 2, and its error is tagged [`ParseFailureKind::GaveUp`] for
/// the same reason the budget failure is: nothing about the *source* was
/// established.
fn slice1_doc<'s>(
    source: &'s str,
    atoms: &[Atom],
    version: RustyfiVersion,
    opts: &CstOptions,
) -> Result<Doc<'s>, ParseFileError> {
    // One builder per grammar (`engine.md` section 9's last paragraph): the
    // engine, the trivia scan, the verifier and the `Doc` IR are shared
    // verbatim, and only the walk forks. 0.1 stays one slice behind, because
    // the 0.0.6 corpus is 162 files against 47 and that is where a layout
    // decision gets its evidence.
    let mut stream = rustyfi_syntax::stream::AtomStream::new(atoms.to_vec());
    let built = match version {
        RustyfiVersion::V0_1 => {
            let file = <rustyfi_syntax::cst_v1::FileV1 as Parse<_>>::parse(&mut stream)
                .map_err(|e| locate(source, &stream, &e))?;
            build01::build(
                source,
                atoms,
                &file,
                opts.tab_spaces,
                build006::SLICE3,
                wrap_budget(opts),
                opts.wrap_inline_text,
            )
        }
        _ => {
            let file = <rustyfi_syntax::cst::File as Parse<_>>::parse(&mut stream)
                .map_err(|e| locate(source, &stream, &e))?;
            build006::build(
                source,
                atoms,
                &file,
                opts.tab_spaces,
                build006::SLICE3,
                wrap_budget(opts),
                opts.wrap_inline_text,
            )
        }
    };
    built.ok_or_else(|| ParseFileError {
        span: rustyfi_syntax::Span::default(),
        message: "the layout builder has no rule for this file".to_string(),
        kind: ParseFailureKind::GaveUp,
    })
}

/// Whether the slice-1 walk stayed in step with the atom stream, for the corpus
/// test that asserts it did. `None` for a buffer that does not lex or parse.
///
/// Not consulted by [`format_cst`], which could not act on it: a drift
/// misattributes *indentation*, and the token stream is identical either way.
/// So the invariant has to be asserted from outside, over real files.
pub fn cst_walk_desync(source: &str, version: RustyfiVersion, opts: &CstOptions) -> Option<usize> {
    let atoms = lex_with_version(source, version).ok()?;
    let mut stream = rustyfi_syntax::stream::AtomStream::new(atoms.clone());
    match version {
        RustyfiVersion::V0_1 => {
            let file = <rustyfi_syntax::cst_v1::FileV1 as Parse<_>>::parse(&mut stream).ok()?;
            Some(build01::walk_desync(source, &atoms, &file, opts.tab_spaces))
        }
        _ => {
            let file = <rustyfi_syntax::cst::File as Parse<_>>::parse(&mut stream).ok()?;
            Some(build006::walk_desync(
                source,
                &atoms,
                &file,
                opts.tab_spaces,
            ))
        }
    }
}

/// Does `after` lex to the same token stream as `before`?
///
/// Slots and payloads, in order, with **one licensed difference** (below).
/// There is otherwise nothing to quotient out: comments and program-area
/// whitespace produce no tokens in the first place, so a difference here is a
/// real difference. This is the same comparison `tests/format.rs` makes, and
/// for the same stated reason — a formatter bug that split `::`, merged `- 1`
/// into `-1`, or turned a `Break` into a `Space` shows up as a difference, and
/// each of those is a document that renders differently.
///
/// # The one licensed difference, and why it is checked rather than assumed
///
/// Slice 6 re-wraps inline text, and a whitespace run inside `{ … }` IS a
/// token whose identity is `Break` if the run starts with a newline and
/// `Space` otherwise (`lexer.rs:1149-1155`). Filling a line therefore turns
/// some `Space`s into `Break`s and some `Break`s into `Space`s — by
/// construction, since that is the edit — so a comparison that refused every
/// slot difference would decline every file the feature touches.
///
/// So `Space` <-> `Break` at index *i* is accepted **iff
/// [`inline::gap_is_reflowable`] clears index *i*** — the same measured
/// predicate the builders consult, re-derived here from the LEXER's own
/// output rather than from the builder's walk. That is what makes this a
/// check and not a rubber stamp: a builder that reflowed a frozen gap, or
/// misread which token a gap's neighbour is, is rejected here by a second
/// computation over a different input. Everything else is unchanged — a
/// different count, a run that stopped existing, a `Char` whose payload
/// moved, a `%` comment that vanished (it lives inside the run's span, so
/// deleting one changes no slot at all — which is why [`inline`] freezes such
/// a run in the builder and why property 2 of the corpus sweep compares these
/// tokens' bytes modulo whitespace).
///
/// The predicate is evaluated on **`before`**, the author's own stream, so a
/// bug that changed a neighbouring `Char` cannot also change the licence it
/// is judged under.
fn same_tokens(before: &str, after: &str, version: RustyfiVersion) -> bool {
    let Ok(a) = lex_with_version(before, version) else {
        return false;
    };
    let Ok(b) = lex_with_version(after, version) else {
        return false;
    };
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(&b).enumerate().all(|(i, (x, y))| {
        if x.slot == y.slot {
            return true;
        }
        matches!(
            (&x.slot, &y.slot),
            (Token::Space, Token::Break) | (Token::Break, Token::Space)
        ) && inline::gap_is_reflowable(&a, i)
    })
}

/// The line terminator this file already uses, taken from its first one.
///
/// Same rule and same reason as `format.rs:636-642`: in slice 0 every existing
/// terminator is copied through untouched, so this only has to avoid being the
/// odd one out. From slice 1, when the renderer invents terminators in program
/// areas, it becomes the file's answer.
fn dominant_newline(source: &str) -> &'static str {
    match source.find('\n') {
        Some(i) if i > 0 && source.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominant_newline_reads_the_files_first_terminator() {
        assert_eq!(dominant_newline("a\nb"), "\n");
        assert_eq!(dominant_newline("a\r\nb"), "\r\n");
        // No terminator at all: the answer is only used when the renderer
        // INVENTS one, and a one-line file that gains a final newline should
        // gain the platform-neutral one rather than a guess.
        assert_eq!(dominant_newline("a"), "\n");
        assert_eq!(dominant_newline(""), "\n");
        // A bare `\r` first: `find('\n')` does not see it, so this answers
        // "\n". Deliberate — a bare-CR file is not a line ending anybody is
        // still writing, and inventing `\r` for it would be worse.
        assert_eq!(dominant_newline("a\rb"), "\n");
    }

    /// `Token::Eoi`'s span always ends at `source.len()`.
    ///
    /// This used to justify calling the slice-0 identity builder's trailing
    /// branch unreachable. That builder is **gone** — tier 2 is
    /// [`crate::format`] now, per `engine.md` section 8 — but the property it
    /// rested on is not the builder's: [`trivia`] and both walks address gaps
    /// as `previous atom's end .. next atom's start`, so an `Eoi` that stopped
    /// short would drop the file's tail on every path, silently. Kept as the
    /// standing statement of that, at the one place it is cheap to check.
    #[test]
    fn eoi_always_ends_at_the_source_length() {
        for src in ["3", "3\n", "3\r\n", "% c", "let x = 1 in x  "] {
            let atoms = lex_with_version(src, RustyfiVersion::V0_0).expect("lexes");
            let last = atoms.last().expect("at least Eoi");
            assert_eq!(last.slot, Token::Eoi, "{src:?}");
            assert_eq!(
                last.span.end.byte,
                src.len(),
                "{src:?}: Eoi does not reach the end, so every gap walk in this \
                 module loses the file's tail"
            );
        }
    }

    /// Tier 2 is [`crate::format`]'s output, not the identity.
    ///
    /// The mutation this test exists to catch is returning `source` unchanged
    /// from [`fall_back`]: the buffer below does not parse (`1+h` lexes as `1`
    /// and the block command `+h`) and carries trailing whitespace, a tab and a
    /// four-blank-line run, none of which the identity builder ever touched.
    #[test]
    fn tier_two_is_the_whitespace_formatters_output() {
        let src = "let a = 1 in   \n\n\n\n\nlet bad = 1+h 2 in\n\tlet b = 3 in b";
        let out = format_cst_outcome(src, RustyfiVersion::V0_0, &CstOptions::default());
        let CstOutcome::FellBack { text, changed, .. } = &out else {
            panic!("expected tier 2, got {out:?}");
        };
        assert!(changed, "tier 2 left the buffer alone");
        assert_eq!(
            text, "let a = 1 in\n\n\nlet bad = 1+h 2 in\n  let b = 3 in b\n",
            "tier 2 did not trim, cap and expand the way `crate::format` does"
        );
    }

    /// A parse failure is never reported as a format.
    ///
    /// The defect this whole change exists for: both arms used to be
    /// `Some(text)`, so `--check` could not tell them apart and exited `0`.
    #[test]
    fn a_parse_failure_is_a_distinct_outcome_from_a_format() {
        let opts = CstOptions::default();
        let ok = "let a = 1+2 in\nlet b = 3+4 in\nb\n";
        let bad = "let a = 1+2 in\nlet bad = 1+h 2 in\nlet b = 3+4 in\nb\n";
        assert!(
            format_cst_outcome(ok, RustyfiVersion::V0_0, &opts).parsed(),
            "the control does not parse, so this test proves nothing"
        );
        let out = format_cst_outcome(bad, RustyfiVersion::V0_0, &opts);
        assert!(!out.parsed(), "a file that does not parse reported as parsed");
        assert!(matches!(out, CstOutcome::FellBack { .. }), "{out:?}");
    }

    /// A buffer that does not LEX still declines, and says which decline it is.
    #[test]
    fn a_buffer_that_does_not_lex_declines() {
        // An unterminated string literal: the mode stack is what fails.
        let out =
            format_cst_outcome("let s = `x in s\n", RustyfiVersion::V0_0, &CstOptions::default());
        assert!(
            matches!(out, CstOutcome::Declined(DeclineReason::DoesNotLex)),
            "{out:?}"
        );
        assert_eq!(out.text(), None, "a decline must carry no bytes");
    }

    /// A give-up is carried out as a give-up, not as a syntax error.
    ///
    /// Reached by construction rather than by finding a pathological file: the
    /// builder's own "no layout rule" answer takes the same road, and both must
    /// arrive tagged [`ParseFailureKind::GaveUp`] so the CLI does not tell a
    /// user their valid file is broken.
    #[test]
    fn give_up_is_not_reported_as_a_syntax_error() {
        let bad = "let a = 1 in\nlet bad = 1+h 2 in\nb\n";
        let out = format_cst_outcome(bad, RustyfiVersion::V0_0, &CstOptions::default());
        let err = out.parse_error().expect("tier 2 carries the failure");
        assert_eq!(
            err.kind,
            ParseFailureKind::Syntax,
            "a genuine syntax error must not be softened into a give-up"
        );
        assert!(
            !err.message.is_empty() && !err.render().contains("gave up"),
            "{err:?}"
        );
    }
}
