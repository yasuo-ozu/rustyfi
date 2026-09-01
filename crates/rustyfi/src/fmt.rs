//! `rustyfi fmt` — the CST formatter's command-line entry point, and the
//! measurement tool the formatter project is built against.
//!
//! The formatting itself is entirely `rustyfi_lsp::format_cst`'s; this module
//! is the argument surface, the file walk, the diff and the exit code. Nothing
//! here decides what a formatted file looks like, which is deliberate: the
//! library is the single implementation shared with the language server and the
//! playground, so a `--check` run in CI and a save in an editor cannot disagree.
//!
//! ## Why `--check` is the point
//!
//! `docs/plans/formatter-cst/config.md` §4.3: the option surface's admission
//! rule demands "two independent failures, counted, evidenced by a file and
//! line count from a named `rustyfi fmt --check` sweep". Without this
//! subcommand every claim about the corpus is unfalsifiable. So `--check`
//! prints a **unified diff**, not a list of filenames — a boolean answers "is
//! it clean", a diff answers "what would it do", and only the second one can
//! settle an argument about a default.
//!
//! It is also an independent check on the formatter's own claims. Slice 0
//! promises byte-for-byte identity on any file that lexes; this code path
//! shares no line with the tests that assert it, so a sweep reporting `0
//! reformatted, 0 declined` over the 209 bundled corpus files is corroboration
//! rather than restatement.
//!
//! ## Exit codes
//!
//! Extending the scheme [`crate::run`] documents (0-5 were taken):
//!
//! | code | meaning |
//! |---|---|
//! | 0 | clean — nothing needs reformatting, nothing was declined, everything parsed |
//! | 1 | `--check` found at least one file that needs reformatting |
//! | 2 | usage |
//! | 5 | filesystem: a path that cannot be read, a file that cannot be written |
//! | 6 | at least one file was DECLINED (it does not lex) |
//! | 7 | at least one file did not PARSE, so only whitespace was normalised |
//!
//! `6` is not folded into `1` because CI has to tell "somebody forgot to run
//! the formatter" from "somebody committed a file that does not lex". The
//! first is a one-command fix; the second is a broken commit.
//!
//! **`7` is new, and it is a THIRD state rather than an extension of `6`.**
//! Extending `6`'s meaning was the alternative and it is wrong on the one
//! distinction that matters to a caller: a declined file produces **no bytes**
//! and is never written, whereas an unparsed file produces `crate::format`'s
//! whitespace-only output, which `rustyfi fmt` **does write**. So after a
//! `6` run the tree is untouched and after a `7` run it is partly formatted —
//! two different things to do next, and one exit code cannot say both. They
//! also differ in how certain the diagnosis is: `6` means the lexer rejected
//! the characters, full stop, while a `7` can be a
//! [`ParseFailureKind::GaveUp`], which explicitly does not claim the file is
//! invalid. What `7` must never be is `0`, which is what it was: a file the
//! formatter could not parse came back byte-identical from the old identity
//! fallback, indistinguishable from an already-formatted file, and `--check`
//! exited `0` over a formatter that had done nothing.
//!
//! Precedence when several apply is `5 > 6 > 7 > 1 > 0`: a path that could not
//! be read makes the whole answer unreliable; a decline (no answer at all) is
//! worse news than a partial answer; and a partial answer is worse news than a
//! pending reformat, which is merely a command nobody ran.
//!
//! ## Never writing a declined file
//!
//! A decline means the formatter has no reliable answer for this buffer, so
//! there is nothing it would be correct to write. That is enforced by the types
//! rather than by a checked code path: [`format_source`] returns
//! `Result<Formatted, Declined>`, [`Formatted`] is constructible only inside
//! that function, and [`write_back`] takes a `&Formatted`. A decline therefore
//! does not carry any bytes for a caller to write by mistake.
//!
//! An UNPARSED file is not a decline: it has bytes, they are safe (they are the
//! whitespace-only formatter's, re-verified for token identity), and refusing
//! to write them would make a mid-typing save do nothing. [`Formatted`]
//! therefore carries the parse failure alongside the bytes rather than
//! withholding them.
//!
//! ## No config file
//!
//! §4.3 proposes `--config-path FILE` (rustfmt's spelling, and explicitly NOT
//! `--config`, which this CLI already uses for the satyrographos registry
//! config — two different files behind one flag name is a difference nobody
//! could explain). No config loader exists yet, so rather than ship a flag that
//! reads a format nobody has specified, this subcommand uses
//! [`CstOptions::default`] and says so in its help text. When the loader lands,
//! the flag is `--config-path`.

use std::path::{Path, PathBuf};

use clap::ArgMatches;
use rustyfi_lsp::{format_cst_outcome, CstOptions, CstOutcome, DeclineReason};
use rustyfi_syntax::{ParseFailureKind, ParseFileError, RustyfiVersion};

/// Where formatted output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emit {
    /// Rewrite each input file in place. The default.
    Files,
    /// Concatenate every input's formatted text onto stdout, writing nothing.
    Stdout,
}

/// The extensions a SATySFi source file has, matching
/// `rustyfi-lsp`'s `workspace::walk` and `rustyfi-syntax`'s
/// `atoms_roundtrip::corpus` so that "the project's source files" means the
/// same set of files everywhere in the tree.
const SOURCE_EXTENSIONS: [&str; 3] = ["saty", "satyh", "satyg"];

/// Directories the project walk never descends into — build output and vendored
/// dependencies, which are not the user's source even when they contain
/// `.satyh` files. Same list as `rustyfi-lsp`'s `workspace::SKIP_DIRS`.
const SKIP_DIRS: [&str; 3] = ["target", "_build", "node_modules"];

/// Output bytes a successful format produced, and how much of a format it was.
///
/// The private fields and the private constructor are the whole point: the only
/// way to obtain one is [`format_source`]'s `Ok` arm, and it is the only thing
/// [`write_back`] will accept. "Never write a file whose formatting declined"
/// is thus a property of the type rather than of a caller that remembered to
/// check a flag.
struct Formatted {
    text: String,
    /// `Some` when the buffer reached **tier 2**: it lexed, it did not parse,
    /// and `text` is the whitespace-only formatter's output rather than a
    /// layout. Carried beside the bytes rather than replacing them, because
    /// those bytes are still an improvement and still safe to write — see the
    /// module doc's "Never writing a declined file".
    unparsed: Option<ParseFileError>,
}

/// The formatter had no reliable answer for this buffer. Carries no bytes, on
/// purpose.
struct Declined(DeclineReason);

impl Declined {
    /// What to print. Both arms are real and they are different bugs: the first
    /// is the user's file, the second is this port's printer.
    fn why(&self) -> &'static str {
        match self.0 {
            DeclineReason::DoesNotLex => {
                "the file does not lex, so there is no token stream to re-emit"
            }
            DeclineReason::VerifierRejected => {
                "the formatter's own output did not lex back to the same tokens, \
                 which is a bug in the formatter — please report it"
            }
        }
    }
}

/// Everything the run needs that is not a path.
struct Config {
    check: bool,
    /// `None` = sniff each file's own generation, the way
    /// `rustyfi_lsp::format_auto` does.
    lang: Option<RustyfiVersion>,
    emit: Emit,
    opts: CstOptions,
}

/// What happened across the whole run, folded into an exit code at the end.
#[derive(Default)]
struct Tally {
    seen: usize,
    /// Files whose formatted bytes differ from what is on disk — **including**
    /// tier-2 files, whose whitespace-only output can differ too. A file can
    /// therefore be counted here and in `unparsed` at once, which is the honest
    /// reading: it needs reformatting AND the formatter could not fully do it.
    reformatted: usize,
    declined: usize,
    /// Files that lexed but did not parse, so only whitespace was normalised.
    unparsed: usize,
    io_errors: usize,
}

impl Tally {
    /// Precedence `5 > 6 > 7 > 1 > 0`, as the module doc explains.
    ///
    /// `check` is a parameter rather than a field because `reformatted` counts
    /// **files whose formatting differs from what is on disk**, which is a
    /// statement about the files and not about whether the run succeeded. Under
    /// `--check` that is a failure, since nothing was written and the tree is
    /// still unformatted; in the in-place mode it is the tool doing its job,
    /// and exiting non-zero there would break every `rustyfi fmt && git commit`
    /// anyone writes.
    fn exit_code(&self, check: bool) -> i32 {
        if self.io_errors > 0 {
            return 5;
        }
        if self.declined > 0 {
            return 6;
        }
        // Not gated on `check`. A file that did not parse was only
        // whitespace-tidied whether or not anything was written, and that is
        // news either way — the whole defect was this state reporting as `0`.
        if self.unparsed > 0 {
            return 7;
        }
        if check && self.reformatted > 0 {
            return 1;
        }
        0
    }
}

pub fn run(m: &ArgMatches) -> i32 {
    let lang = match crate::parse_lang_flag(m.get_one::<String>("lang").map(String::as_str)) {
        Ok(lang) => lang,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let emit = match m.get_one::<String>("emit").map(String::as_str) {
        // `--check` writes nothing at all, so it does not reach `write_back`
        // and the value here is never consulted; clap already refuses the two
        // flags together (`--emit` conflicts with `--check`).
        Some("stdout") => Emit::Stdout,
        Some("files") | None => Emit::Files,
        Some(other) => {
            // Unreachable: clap's `value_parser` has already rejected anything
            // else. Kept so that adding a value to the parser without adding
            // it here fails loudly instead of silently formatting to a file.
            eprintln!("error: --emit: unknown destination `{other}`");
            return 2;
        }
    };
    let cfg = Config {
        check: m.get_flag("check"),
        lang,
        emit,
        // No config file: see the module doc's "No config file".
        opts: CstOptions::default(),
    };

    let args: Vec<PathBuf> = m
        .get_many::<PathBuf>("paths")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();

    // `-` is stdin mode, and it is exclusive: a run that mixed stdin with named
    // files would have to interleave one anonymous buffer's output with
    // in-place rewrites of the others, and there is no sensible reading of that.
    let stdin_mode = args.iter().any(|p| p.as_os_str() == "-");
    if stdin_mode {
        if args.len() > 1 {
            eprintln!("error: `-` reads stdin and cannot be combined with file paths");
            return 2;
        }
        return run_stdin(&cfg);
    }

    let files = match collect_files(&args) {
        Ok(files) => files,
        Err(code) => return code,
    };

    let mut tally = Tally::default();
    for path in &files {
        run_one(path, &cfg, &mut tally);
    }
    flush_stdout();
    report(&tally, &cfg);
    tally.exit_code(cfg.check)
}

/// `rustyfi fmt -`: format stdin onto stdout.
///
/// `--emit` is irrelevant here (there is no file to write back to) and
/// `--check` still means "write nothing": it prints the diff and reports
/// through the exit code, exactly as it does for a named file.
fn run_stdin(cfg: &Config) -> i32 {
    let mut source = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut source) {
        eprintln!("error: reading stdin: {e}");
        return 5;
    }
    match format_source(&source, cfg) {
        Err(declined) => {
            eprintln!("error: <stdin>: declined — {}", declined.why());
            6
        }
        Ok(formatted) => {
            // Reported before the diff so it precedes the thing it qualifies,
            // and on stderr so `--check`'s stdout stays a pure diff.
            if let Some(e) = &formatted.unparsed {
                eprintln!("{}", unparsed_note("<stdin>", e));
            }
            let mut code = match cfg.check {
                true => match check_verdict("<stdin>", &source, &formatted) {
                    Some(diff) => {
                        print!("{diff}");
                        1
                    }
                    None => 0,
                },
                false => {
                    print!("{}", formatted.text);
                    0
                }
            };
            // `7 > 1`, the same precedence `Tally::exit_code` applies. Without
            // this, `rustyfi fmt --check -` exits 0 on a buffer it could not
            // parse and merely tidied — the defect, one code path over.
            if formatted.unparsed.is_some() {
                code = 7;
            }
            flush_stdout();
            code
        }
    }
}

/// The one-line warning an unparsed file gets, on stderr.
///
/// Says the tier, the position, and what was done instead — in that order,
/// because "only whitespace was normalised" is the part that changes what the
/// reader does next. [`ParseFailureKind::GaveUp`] gets its own sentence rather
/// than being folded in: it explicitly does not claim the file is invalid, and
/// telling somebody their valid 5000-line document has a syntax error is the
/// same silent misreport this whole change is about, one level up.
fn unparsed_note(label: &str, e: &ParseFileError) -> String {
    let tail = match e.kind {
        ParseFailureKind::GaveUp => {
            " — the parser ran out of backtracking budget rather than finding an \
             error, so the file may well be valid; only whitespace was normalised"
        }
        _ => " — only whitespace was normalised, not the layout",
    };
    format!("warning: {label}: {}: {}{tail}", e.span, e.render())
}

/// Flush stdout before the process exits.
///
/// `main` leaves through `std::process::exit`, which does NOT run the runtime's
/// end-of-`main` stdout cleanup. Rust's stdout is a `LineWriter`, so everything
/// ending in a newline has already gone out — but a formatted file whose last
/// line has no terminator would lose that line, which for `--emit stdout` and
/// for stdin mode means silently truncating the user's document.
fn flush_stdout() {
    let _ = std::io::Write::flush(&mut std::io::stdout().lock());
}

/// Format one file, recording the outcome in `tally`.
fn run_one(path: &Path, cfg: &Config, tally: &mut Tally) {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {e}", path.display());
            tally.io_errors += 1;
            return;
        }
    };
    tally.seen += 1;
    let formatted = match format_source(&source, cfg) {
        Ok(f) => f,
        Err(declined) => {
            eprintln!("error: {}: declined — {}", path.display(), declined.why());
            tally.declined += 1;
            return;
        }
    };
    if let Some(e) = &formatted.unparsed {
        eprintln!("{}", unparsed_note(&path.display().to_string(), e));
        tally.unparsed += 1;
    }
    if cfg.check {
        if let Some(diff) = check_verdict(&path.display().to_string(), &source, &formatted) {
            tally.reformatted += 1;
            print!("{diff}");
        }
        return;
    }
    // Counted before the emit decision, because it describes the FILE — how
    // many inputs were not already formatted — and stays comparable between a
    // `--check` run and the in-place run that fixes it.
    if formatted.text != source {
        tally.reformatted += 1;
    }
    match cfg.emit {
        Emit::Stdout => print!("{}", formatted.text),
        Emit::Files => {
            if formatted.text == source {
                // Identical bytes: leave the file, and its mtime, alone. Every
                // build system in the world watches mtimes.
                return;
            }
            if let Err(e) = write_back(path, &formatted) {
                eprintln!("error: {}: {e}", path.display());
                tally.io_errors += 1;
            }
        }
    }
}

/// `--check`'s verdict for one file: `Some(diff)` if it needs reformatting,
/// `None` if it is already formatted.
///
/// Extracted from [`run_one`] so that the "needs reformatting" decision — and
/// therefore exit code `1` — is testable **without** an input the formatter
/// actually changes. Under slice 0 there is no such input, by design: it is a
/// provable no-op, so no file on disk can drive the exit-`1` path end to end
/// and a `--check` that always reported "clean" would pass a CLI test suite
/// unnoticed. This function plus [`Tally::exit_code`] cover between them the
/// whole of that contract, and both are unit-tested below.
fn check_verdict(label: &str, source: &str, formatted: &Formatted) -> Option<String> {
    if formatted.text == source {
        return None;
    }
    Some(unified_diff(label, source, &formatted.text))
}

/// Format `source`, choosing the generation as configured.
///
/// The ONLY constructor of a [`Formatted`], which is what makes "a declined
/// file is never written" a type-level property.
///
/// With no `--lang`, this is `rustyfi_lsp::format_auto`'s rule applied to the
/// CST formatter: prefer the generation the file's own headers name, then try
/// the other, and **let the attempt be the test**. The fallback matters because
/// the sniffer reads header lines only: a 0.1 file whose headers are
/// 0.0.6-shaped (or absent) lexes only under 0.1, and refusing it because a
/// header walk guessed wrong would be a formatter that declines files it can
/// format perfectly.
///
/// The fallback is now driven by `CstOutcome::rank` rather than by
/// `Option::or_else`, and that is `engine.md` section 8's rule rather than a
/// refinement of it: "treat a tier-2 outcome under one generation as a reason
/// to try the other before settling". With `Option` the two were the same
/// answer, so a 0.1 file with 0.0.6-shaped headers LEXED under 0.0.6, failed to
/// parse, and was accepted as tier 2 — the right generation never tried.
fn format_source(source: &str, cfg: &Config) -> Result<Formatted, Declined> {
    let attempt = |v: RustyfiVersion| format_cst_outcome(source, v, &cfg.opts);
    let out = match cfg.lang {
        // `--lang` pins it: an explicit generation is an assertion about the
        // file, and silently formatting it as the other one would hide the
        // user's mistake rather than report it.
        Some(v) => attempt(v),
        None => {
            let preferred = rustyfi_syntax::sniff_version(source).unwrap_or(RustyfiVersion::V0_0);
            // `RustyfiVersion` is non-exhaustive, so this is a two-way swap
            // with an explicit default rather than a match a third generation
            // would silently break.
            let fallback = match preferred == RustyfiVersion::V0_1 {
                true => RustyfiVersion::V0_0,
                false => RustyfiVersion::V0_1,
            };
            let first = attempt(preferred);
            match first.rank() {
                // A full format under the sniffed generation is the answer; the
                // second parse is not worth its latency.
                2 => first,
                _ => {
                    let second = attempt(fallback);
                    // Ties keep the SNIFFED generation, which is the one the
                    // file's own headers asked for.
                    match second.rank() > first.rank() {
                        true => second,
                        false => first,
                    }
                }
            }
        }
    };
    match out {
        CstOutcome::Declined(why) => Err(Declined(why)),
        CstOutcome::Formatted(text) | CstOutcome::AlreadyFormatted(text) => Ok(Formatted {
            text,
            unparsed: None,
        }),
        CstOutcome::FellBack { text, error, .. } => Ok(Formatted {
            text,
            unparsed: Some(error),
        }),
    }
}

/// Replace `path`'s contents with formatted bytes.
///
/// Takes a `&Formatted` and nothing else, which is the enforcement described in
/// the module doc: there is no way to call this with a declined file's source,
/// because a decline produces no `Formatted` at all.
fn write_back(path: &Path, formatted: &Formatted) -> std::io::Result<()> {
    std::fs::write(path, &formatted.text)
}

/// Expand the command line's paths into the list of files to format.
///
/// A named file is taken as given, whatever its extension — naming it is the
/// user saying it is SATySFi source. A named DIRECTORY is walked for the
/// [`SOURCE_EXTENSIONS`], because a directory is a wildcard and a wildcard that
/// swept up `README.md` would be surprising.
///
/// With no paths at all, "the project" is the directory holding the nearest
/// `Satyristes` at or above the working directory — the same manifest
/// discovery `satyrographos install` and `update` use (`crate::find_manifest`).
/// **With no manifest that is a usage error**, not a silent walk from the
/// working directory: a formatter that rewrites files in place must never guess
/// at which files, and the difference between "the project" and "whatever I
/// happen to be standing in" is a `rm -rf`-class mistake.
fn collect_files(args: &[PathBuf]) -> Result<Vec<PathBuf>, i32> {
    if args.is_empty() {
        let Some(manifest) = crate::find_manifest() else {
            eprintln!(
                "error: no PATHS given and no `Satyristes` found at or above the working \
                 directory, so there is no project to format. Name the files or directories \
                 to format explicitly."
            );
            return Err(2);
        };
        let root = manifest
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut out = Vec::new();
        walk(&root, &mut out);
        out.sort();
        return Ok(out);
    }
    let mut out = Vec::new();
    for arg in args {
        let meta = match std::fs::metadata(arg) {
            Ok(md) => md,
            Err(e) => {
                eprintln!("error: {}: {e}", arg.display());
                return Err(5);
            }
        };
        if meta.is_dir() {
            let mut found = Vec::new();
            walk(arg, &mut found);
            found.sort();
            out.extend(found);
        } else {
            out.push(arg.clone());
        }
    }
    Ok(out)
}

/// Every SATySFi source file under `dir`, recursively.
///
/// Skips dotted entries and [`SKIP_DIRS`], and uses `file_type` rather than
/// `is_dir` so a symlink loop cannot make the walk run forever.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(t) if t.is_file() => {
                if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| SOURCE_EXTENSIONS.contains(&e))
                {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
}

/// The one-line summary, on **stderr** so that `--check`'s stdout stays a pure
/// diff a pipe can consume. Suppressed when nothing was looked at, since the
/// usage or filesystem error already said why.
fn report(tally: &Tally, cfg: &Config) {
    if tally.seen == 0 && tally.io_errors == 0 {
        eprintln!("no SATySFi source files found");
        return;
    }
    let verb = match cfg.check {
        true => "need reformatting",
        false => "reformatted",
    };
    eprintln!(
        "{} file(s) checked: {} {verb}, {} declined, {} unparsed",
        tally.seen, tally.reformatted, tally.declined, tally.unparsed
    );
}

// ---------------------------------------------------------------------------
// A line diff
// ---------------------------------------------------------------------------

/// Unified diff of `old` against `new`, three lines of context.
///
/// Hand-rolled rather than pulled from a crate: the workspace has no diff
/// dependency, and this is the whole of what a `--check` mode needs. `similar`
/// or `diff` would be a new dependency in the compiler's binary for sixty lines
/// of Myers-adjacent bookkeeping.
///
/// The algorithm is plain LCS over lines, after trimming the common prefix and
/// suffix — which is what keeps it affordable, since the interesting case for a
/// formatter is a large file with a small change. [`LCS_CELL_BUDGET`] caps the
/// quadratic for the pathological remainder; past it the differing middle is
/// reported as one delete-then-insert hunk, which is honest (it IS a valid edit
/// script) and merely less pretty.
fn unified_diff(label: &str, old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let ops = diff_ops(&a, &b);
    let mut out = String::new();
    if ops.iter().all(|op| matches!(op, Op::Keep(_))) {
        return out;
    }
    out.push_str(&format!("--- {label}\n+++ {label} (formatted)\n"));
    for hunk in hunks(&ops) {
        out.push_str(&hunk);
    }
    out
}

/// One line of the edit script. The payload is the line's text; the `Keep`
/// variant is context.
enum Op<'s> {
    Keep(&'s str),
    Del(&'s str),
    Ins(&'s str),
}

/// Above this many DP cells the exact LCS is abandoned. 4M cells is a 2000x2000
/// differing region, which no formatter diff over real source reaches; a file
/// that does is one the reader was never going to read line by line anyway.
const LCS_CELL_BUDGET: usize = 4_000_000;

fn diff_ops<'s>(a: &[&'s str], b: &[&'s str]) -> Vec<Op<'s>> {
    // Common prefix and suffix, which for a formatter diff is almost the whole
    // file. Both are emitted as `Keep`, so hunk selection still trims them to
    // context width.
    let mut lead = 0;
    while lead < a.len() && lead < b.len() && a[lead] == b[lead] {
        lead += 1;
    }
    let mut trail = 0;
    while trail < a.len() - lead && trail < b.len() - lead
        && a[a.len() - 1 - trail] == b[b.len() - 1 - trail]
    {
        trail += 1;
    }
    let (am, bm) = (&a[lead..a.len() - trail], &b[lead..b.len() - trail]);

    let mut ops: Vec<Op<'s>> = a[..lead].iter().copied().map(Op::Keep).collect();
    if am.len().saturating_mul(bm.len()) > LCS_CELL_BUDGET {
        ops.extend(am.iter().copied().map(Op::Del));
        ops.extend(bm.iter().copied().map(Op::Ins));
    } else {
        ops.extend(lcs_script(am, bm));
    }
    ops.extend(a[a.len() - trail..].iter().copied().map(Op::Keep));
    ops
}

/// Textbook LCS-length table, then one backtrack to turn it into an edit
/// script. `table[i][j]` is the LCS length of `a[i..]` and `b[j..]`, so the
/// backtrack runs forward and the script comes out in source order without a
/// reversal.
fn lcs_script<'s>(a: &[&'s str], b: &[&'s str]) -> Vec<Op<'s>> {
    let (n, m) = (a.len(), b.len());
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = match a[i] == b[j] {
                true => table[i + 1][j + 1] + 1,
                false => table[i + 1][j].max(table[i][j + 1]),
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op::Keep(a[i]));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push(Op::Del(a[i]));
            i += 1;
        } else {
            ops.push(Op::Ins(b[j]));
            j += 1;
        }
    }
    ops.extend(a[i..].iter().copied().map(Op::Del));
    ops.extend(b[j..].iter().copied().map(Op::Ins));
    ops
}

/// How many unchanged lines surround a change in the output.
const CONTEXT: usize = 3;

/// Group the edit script into unified-diff hunks with `@@` headers.
///
/// A run of more than `2 * CONTEXT` consecutive `Keep`s ends a hunk; anything
/// shorter is kept, because two changes three lines apart read better as one
/// hunk than as two.
fn hunks(ops: &[Op<'_>]) -> Vec<String> {
    // Index of every op that is a change, then merge those into ranges that
    // include the context around them.
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, Op::Keep(_)))
        .map(|(i, _)| i)
        .collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &i in &changed {
        let lo = i.saturating_sub(CONTEXT);
        let hi = (i + CONTEXT + 1).min(ops.len());
        match ranges.last_mut() {
            Some(last) if lo <= last.1 => last.1 = hi,
            _ => ranges.push((lo, hi)),
        }
    }

    // Running 1-based line numbers in each file, so a hunk header can be
    // written when its range is reached.
    let mut out = Vec::with_capacity(ranges.len());
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    let mut cursor = 0usize;
    for (lo, hi) in ranges {
        for op in &ops[cursor..lo] {
            match op {
                Op::Keep(_) => {
                    old_line += 1;
                    new_line += 1;
                }
                Op::Del(_) => old_line += 1,
                Op::Ins(_) => new_line += 1,
            }
        }
        cursor = hi;
        let (mut old_len, mut new_len) = (0usize, 0usize);
        let mut body = String::new();
        for op in &ops[lo..hi] {
            match op {
                Op::Keep(l) => {
                    old_len += 1;
                    new_len += 1;
                    body.push_str(&format!(" {l}\n"));
                }
                Op::Del(l) => {
                    old_len += 1;
                    body.push_str(&format!("-{l}\n"));
                }
                Op::Ins(l) => {
                    new_len += 1;
                    body.push_str(&format!("+{l}\n"));
                }
            }
        }
        // Unified diff numbers an empty side as starting at line 0, which is
        // what `patch` expects for a pure insertion or deletion.
        let old_start = match old_len {
            0 => 0,
            _ => old_line,
        };
        let new_start = match new_len {
            0 => 0,
            _ => new_line,
        };
        out.push(format!(
            "@@ -{old_start},{old_len} +{new_start},{new_len} @@\n{body}"
        ));
        old_line += old_len;
        new_line += new_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // Exit codes
    //
    // The entire contract of a `--check` mode, and — because slice 0 is a
    // provable no-op — the part of it that `tests/fmt_cli.rs` cannot reach
    // through a file on disk for the `1` case. Every assertion here was
    // mutation-tested by making the mutation and watching it fail.
    // -----------------------------------------------------------------

    fn tally(reformatted: usize, declined: usize, io_errors: usize) -> Tally {
        Tally {
            seen: 1,
            reformatted,
            declined,
            unparsed: 0,
            io_errors,
        }
    }

    /// A tally with an unparsed file in it. Separate constructor rather than a
    /// fifth positional argument, so every existing assertion above keeps
    /// saying what it said.
    fn unparsed_tally(reformatted: usize, unparsed: usize) -> Tally {
        Tally {
            seen: 1,
            reformatted,
            declined: 0,
            unparsed,
            io_errors: 0,
        }
    }

    /// `true`/`false` here is `--check`; both are always asserted, because the
    /// difference between them IS the contract.
    const CHECK: bool = true;
    const IN_PLACE: bool = false;

    #[test]
    fn a_clean_run_exits_zero() {
        assert_eq!(tally(0, 0, 0).exit_code(CHECK), 0);
        assert_eq!(tally(0, 0, 0).exit_code(IN_PLACE), 0);
    }

    #[test]
    fn a_file_needing_reformatting_exits_one_under_check() {
        assert_eq!(tally(1, 0, 0).exit_code(CHECK), 1);
    }

    /// The other side of it: a file the IN-PLACE run reformatted is the tool
    /// succeeding, so `rustyfi fmt && git commit` must not be broken by it.
    #[test]
    fn a_file_reformatted_in_place_exits_zero() {
        assert_eq!(tally(1, 0, 0).exit_code(IN_PLACE), 0);
    }

    #[test]
    fn a_declined_file_exits_six() {
        assert_eq!(tally(0, 1, 0).exit_code(CHECK), 6);
        assert_eq!(
            tally(0, 1, 0).exit_code(IN_PLACE),
            6,
            "a decline is news in either mode: nothing was written for that file"
        );
    }

    /// **The defect, as an exit code.** A file that lexes but does not parse
    /// was formatted by nobody, and the run used to report `0`.
    #[test]
    fn an_unparsed_file_exits_seven() {
        assert_eq!(unparsed_tally(0, 1).exit_code(CHECK), 7);
        assert_eq!(
            unparsed_tally(0, 1).exit_code(IN_PLACE),
            7,
            "an unparsed file is news in either mode: what was written is \
             whitespace only, not a format"
        );
    }

    /// `7` is not a synonym for `1`. A tier-2 file whose whitespace-only output
    /// also differs from disk is BOTH, and the worse of the two wins.
    #[test]
    fn an_unparsed_file_that_also_needs_reformatting_still_exits_seven() {
        assert_eq!(unparsed_tally(1, 1).exit_code(CHECK), 7);
    }

    #[test]
    fn a_filesystem_failure_exits_five() {
        assert_eq!(tally(0, 0, 1).exit_code(CHECK), 5);
        assert_eq!(tally(0, 0, 1).exit_code(IN_PLACE), 5);
    }

    /// The precedence, stated as pairwise comparisons rather than as a
    /// re-reading of the function: `5 > 6 > 7 > 1`.
    #[test]
    fn the_worse_news_wins() {
        assert_eq!(
            tally(1, 1, 0).exit_code(CHECK),
            6,
            "a decline outranks a diff"
        );
        assert_eq!(
            Tally {
                seen: 1,
                reformatted: 1,
                declined: 1,
                unparsed: 1,
                io_errors: 0,
            }
            .exit_code(CHECK),
            6,
            "a decline outranks an unparsed file: no bytes at all is worse \
             news than partial bytes"
        );
        assert_eq!(
            tally(0, 1, 1).exit_code(CHECK),
            5,
            "an I/O error outranks both"
        );
        assert_eq!(tally(1, 0, 1).exit_code(CHECK), 5);
        assert_eq!(tally(1, 1, 1).exit_code(CHECK), 5);
        assert_eq!(tally(1, 1, 1).exit_code(IN_PLACE), 5);
    }

    /// The other half of the exit-`1` contract: that a difference is actually
    /// NOTICED. `Formatted` is constructed by hand here, which the module's
    /// type-level guarantee allows — the guarantee is that nothing OUTSIDE this
    /// module can make one, and `check_verdict` writes no files regardless.
    #[test]
    fn check_reports_a_difference_and_only_a_difference() {
        let same = Formatted {
            text: "a\nb\n".to_string(),
            unparsed: None,
        };
        assert!(
            check_verdict("f", "a\nb\n", &same).is_none(),
            "identical bytes are clean"
        );
        let changed = Formatted {
            text: "a\n  b\n".to_string(),
            unparsed: None,
        };
        let diff = check_verdict("f", "a\nb\n", &changed).expect("a difference is reported");
        assert!(diff.contains("-b\n") && diff.contains("+  b\n"), "{diff}");
    }

    /// A give-up must not read as "your file is broken".
    ///
    /// `ParseFailureKind::GaveUp` says explicitly that the file may well be
    /// valid, so the note it gets says the parser ran out of budget and does
    /// not use the words "parse error". Mutation-tested by collapsing the two
    /// arms into one, which fails on the second half of each assertion.
    #[test]
    fn a_give_up_is_worded_differently_from_a_syntax_error() {
        let syntax = ParseFileError {
            span: rustyfi_syntax::Span::default(),
            message: "unexpected `+h`".to_string(),
            kind: ParseFailureKind::Syntax,
        };
        let gave_up = ParseFileError {
            span: rustyfi_syntax::Span::default(),
            message: "this file needs more backtracking than the parser allows".to_string(),
            kind: ParseFailureKind::GaveUp,
        };
        let a = unparsed_note("f", &syntax);
        assert!(a.contains("parse error"), "{a}");
        assert!(!a.contains("may well be valid"), "{a}");
        let b = unparsed_note("f", &gave_up);
        assert!(b.contains("may well be valid"), "{b}");
        assert!(!b.contains("parse error"), "{b}");
        for note in [&a, &b] {
            assert!(
                note.contains("only whitespace was normalised"),
                "every tier-2 note must say what WAS done: {note}"
            );
        }
    }

    /// `format_source` routes a non-parsing buffer to tier 2 and says so.
    ///
    /// The end-to-end statement of the defect at the CLI's own boundary: the
    /// bytes come back (so a mid-typing save still tidies), and `unparsed`
    /// carries the reason (so the run cannot exit `0`).
    #[test]
    fn format_source_marks_a_non_parsing_buffer_unparsed() {
        let cfg = Config {
            check: false,
            lang: None,
            emit: Emit::Files,
            opts: CstOptions::default(),
        };
        let good = format_source("let a = 1 in a\n", &cfg).unwrap_or_else(|_| panic!("declined"));
        assert!(good.unparsed.is_none(), "the control did not parse");

        let bad = format_source("let a = 1 in   \n\n\n\nlet b = 1+h 2 in b", &cfg)
            .unwrap_or_else(|_| panic!("declined a buffer that lexes"));
        let e = bad.unparsed.as_ref().expect("tier 2 is reported");
        assert_ne!(e.kind, ParseFailureKind::Lex);
        assert_eq!(
            bad.text, "let a = 1 in\n\n\nlet b = 1+h 2 in b\n",
            "tier 2 must be `crate::format`'s output, not the identity"
        );
    }

    #[test]
    fn an_unchanged_text_diffs_to_nothing() {
        assert_eq!(unified_diff("f", "a\nb\nc\n", "a\nb\nc\n"), "");
    }

    #[test]
    fn a_one_line_change_is_one_hunk_with_context() {
        let d = unified_diff("f", "a\nb\nc\nd\ne\n", "a\nb\nX\nd\ne\n");
        assert!(d.starts_with("--- f\n+++ f (formatted)\n@@ "), "{d}");
        assert!(d.contains("-c\n"), "{d}");
        assert!(d.contains("+X\n"), "{d}");
        assert!(d.contains(" a\n") && d.contains(" e\n"), "{d}");
    }

    #[test]
    fn distant_changes_become_separate_hunks() {
        let old: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l1\n", "L1\n").replace("l38\n", "L38\n");
        let d = unified_diff("f", &old, &new);
        assert_eq!(d.matches("@@ -").count(), 2, "{d}");
    }

    #[test]
    fn a_pure_insertion_numbers_the_empty_side_zero() {
        let d = unified_diff("f", "", "a\n");
        assert!(d.contains("@@ -0,0 +1,1 @@\n+a\n"), "{d}");
    }

    /// The budget escape hatch still produces a valid script, not a panic and
    /// not an empty diff.
    #[test]
    fn a_change_past_the_cell_budget_degrades_to_one_hunk() {
        let a: Vec<String> = (0..2500).map(|i| format!("a{i}")).collect();
        let b: Vec<String> = (0..2500).map(|i| format!("b{i}")).collect();
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        let b: Vec<&str> = b.iter().map(String::as_str).collect();
        let ops = diff_ops(&a, &b);
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Del(_))).count(), 2500);
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Ins(_))).count(), 2500);
    }
}
