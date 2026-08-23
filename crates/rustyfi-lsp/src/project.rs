//! Whole-program analysis: resolve the buffer's real program, then typecheck
//! it.
//!
//! [`crate::analyze`] stops at parsing because a detached buffer has no
//! program around it. This module resolves the same program the compiler would
//! (`rustyfi_loader::load`) and runs the same elaborate → typecheck →
//! `:>`-check front half (`rustyfi_lang::check_document_program`), stopping
//! before the closure tree, the font store and the fixpoint.
//!
//! Everything here needs the filesystem, so it is behind the `typecheck`
//! feature and never reaches the wasm-safe [`crate::analyze`] seam.
//!
//! Four things about it are not obvious:
//!
//! **The buffer overrides its own file.** The server holds text the disk has
//! not seen; the loader reads paths. `LoadOptions::sources` reconciles them:
//! `BufferSources` answers the loader's filesystem questions itself for the
//! entry path and delegates everything else to `std::fs`. A buffer with no
//! path (an `untitled:` URI) is not resolvable and stays at parse-only.
//!
//! **A library is checked as a dependency of a synthetic document**, because
//! `elaborate_program` rejects a file with no document expression. This module
//! builds — in memory, never on disk — a stub entry beside the library
//! carrying *the library's own headers* and a `()` body, and splices the
//! buffer's parsed CST in as the last dependency. So the library's
//! `@require:`s resolve as they do in real use, its `@stage:` header still
//! applies (the merge reads it off the file, and the buffer is a dependency
//! here, not the entry), and diagnostic spans are offsets into the buffer's
//! own text. Not equivalent to compiling the library for real — see
//! [`CheckOptions::check_libraries`].
//!
//! **Resolution failure degrades to parse-only.** No library root, an
//! uninstalled package, an `@import:` typo: a wall of "cannot resolve" on
//! every keystroke is the worse-than-nothing failure mode this crate exists to
//! avoid. The load error goes in [`Analysis::note`] and no diagnostic is
//! produced.
//!
//! **A span that is not this buffer's is not shown as if it were.** A
//! `Span` carries no file identity and the program is a merge of many files,
//! so a type error's span may be an offset into a *dependency*.
//! `belongs_to_buffer` tests the span's `(line, col, byte)` triple against
//! the buffer; when it does not hold up, the diagnostic goes at the top of the
//! file and says so.

use std::path::{Path, PathBuf};

use rustyfi_loader::{FileOrigin, LoadMode, LoadOptions, LoadedCst, LoadedFile, SourceProvider};
use rustyfi_syntax::span::{Loc, Span};
use rustyfi_syntax::RustyfiVersion;

use crate::analysis::{self, Parsed};
use crate::line_index::LineIndex;
use crate::{Diag, Severity};

/// How to resolve the program around a buffer.
#[derive(Debug, Default, Clone)]
pub struct CheckOptions {
    /// Force a generation instead of detecting one (`rustyfi lsp --lang 0.1`).
    pub lang: Option<RustyfiVersion>,
    /// The library-root search path, highest priority first. When non-empty
    /// the first entry is the load's `lib_root` and the rest its
    /// `fallback_roots`, and [`Self::discover_roots`] is not consulted at
    /// all — the CLI's rule: a *named* root is exactly that one root, or a
    /// build depends on what happens to be installed on the machine.
    pub lib_roots: Vec<PathBuf>,
    /// How to find library roots when none was named: given the document's own
    /// directory, return the whole search path.
    ///
    /// A function pointer rather than a dependency, because discovery lives in
    /// `rustyfi-satyrographos`, which pulls in tar/flate2/sha2/TLS — too much
    /// for an editor front end. The `rustyfi lsp` binary already links it and
    /// passes `sg::roots::discover_all` in.
    pub discover_roots: Option<fn(&Path) -> Vec<PathBuf>>,
    /// Whether to check a *library* buffer, by the synthetic-document route
    /// this module's doc comment describes.
    ///
    /// **Off by default**, not because the route does not work but because
    /// SATySFi's global-merge module model lets a library use a module it
    /// never `@require:`s (`satysfi-base`'s `tabular2.satyh` calls
    /// `Color.black` and requires only `list`/`table`), leaving it to
    /// whichever document pulls it in to have required that package. Checked
    /// alone, such a valid file reports an unbound name that is not a
    /// mistake — a red squiggle on a file that compiles.
    ///
    /// Opt in with `rustyfi lsp --check-libraries` or
    /// `initializationOptions.checkLibraries`. Parse diagnostics are produced
    /// for a library either way; this only controls the whole-program tier.
    pub check_libraries: bool,
}

/// How far the analysis got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// Lex + parse only — either because the buffer does not parse (in which
    /// case that is the diagnostic), or because the program around it could
    /// not be resolved ([`Analysis::note`] says why).
    Parse,
    /// The whole program was resolved, elaborated and typechecked.
    Program,
}

/// What [`check`] found.
#[derive(Debug, Clone)]
pub struct Analysis {
    /// The generation the buffer was read as — after the ambiguity re-check,
    /// so this is what it actually parses as, not merely what was sniffed.
    pub version: RustyfiVersion,
    /// The diagnostics, in the same zero-based UTF-16 coordinates
    /// [`crate::analyze`] produces.
    pub diagnostics: Vec<Diag>,
    /// How far the analysis got.
    pub depth: Depth,
    /// Why [`Depth::Parse`] rather than [`Depth::Program`], when the buffer
    /// itself parsed cleanly. One line, no trailing newline. Never a
    /// diagnostic: it describes the *analysis*, not the user's file.
    pub note: Option<String>,
    /// How many files the resolved program had, entry included. `0` when the
    /// analysis never got that far, which lets a test prove the dependencies
    /// really were loaded rather than the check passing vacuously.
    pub files: usize,
}

impl Analysis {
    fn parse_only(version: RustyfiVersion, diagnostics: Vec<Diag>) -> Analysis {
        Analysis {
            version,
            diagnostics,
            depth: Depth::Parse,
            note: None,
            files: 0,
        }
    }

    fn degraded(version: RustyfiVersion, note: impl Into<String>) -> Analysis {
        Analysis {
            version,
            diagnostics: Vec::new(),
            depth: Depth::Parse,
            note: Some(note.into()),
            files: 0,
        }
    }
}

/// The stack the analysis runs on.
///
/// The parser and the elaborator recurse per syntactic nesting level, and the
/// CLI already spawns its whole run on a 256 MB stack because a real ~300-line
/// document overflows the default 8 MB main-thread one. A language server is
/// handed the same documents on whatever thread the client's I/O loop is on,
/// so the analysis brings its own stack.
const ANALYSIS_STACK: usize = 256 * 1024 * 1024;

/// Analyse `source` as the file at `path`: parse it, resolve the program
/// around it, and typecheck that.
///
/// `path` is the buffer's own path — the entry document's, or the library's.
/// It does not have to exist on disk (an unsaved buffer does not), but its
/// directory does, since that is what `@import:` resolves against.
///
/// Never panics and never overflows the caller's stack: the analysis runs on
/// its own 256 MB thread, and a panic inside it becomes a degraded result
/// rather than taking the server's I/O loop down.
pub fn check(path: &Path, source: &str, opts: &CheckOptions) -> Analysis {
    let (version, parsed) = analysis::parse_with(source, opts.lang);
    let parsed = match parsed {
        Ok(parsed) => parsed,
        // A buffer that does not parse gets the parse diagnostic and nothing
        // else; a second, derived complaint next to the real one is noise.
        Err(failure) => return Analysis::parse_only(version, vec![failure.into_diag(source)]),
    };
    if matches!(parsed, Parsed::None) {
        return Analysis::parse_only(version, Vec::new());
    }
    if !parsed.is_document() && !opts.check_libraries {
        return Analysis::degraded(
            version,
            "this file is a library, not a document; whole-program checking of \
             libraries is off (see CheckOptions::check_libraries)",
        );
    }
    // `use`-family headers select Envelopes packaging, which reads its configs
    // through `std::fs` directly; the loader refuses a `SourceProvider`
    // alongside it, so there is no seam for the unsaved buffer.
    if rustyfi_syntax::sniff_headers(source).envelope_headers {
        return Analysis::degraded(
            version,
            "`use` headers select Envelopes packaging, which resolves dependencies \
             from a pre-solved rustyfi-deps.yaml rather than from a library root",
        );
    }

    let result = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("rustyfi-lsp-check".into())
            .stack_size(ANALYSIS_STACK)
            .spawn_scoped(scope, || resolve_and_check(path, source, version, parsed, opts))
            .expect("failed to spawn the analysis thread")
            .join()
    });
    match result {
        Ok(analysis) => analysis,
        // The front half is the compiler's own and carries `unreachable!`
        // assertions about shapes the loader is supposed to have ruled out. A
        // buffer that finds one costs this analysis, not the editor's whole
        // diagnostics pane.
        Err(_panic) => Analysis::degraded(
            version,
            "the whole-program analysis panicked; only parse diagnostics are available \
             for this buffer",
        ),
    }
}

/// [`check`]'s body, on the analysis thread: everything from here down may
/// touch the filesystem and recurse deeply.
fn resolve_and_check(
    path: &Path,
    source: &str,
    version: RustyfiVersion,
    parsed: Parsed,
    opts: &CheckOptions,
) -> Analysis {
    let buffer_path = normalize(path);
    // A document is its own entry; a library is spliced in as the last
    // dependency of a stub document carrying its headers.
    let library = (!parsed.is_document()).then(|| stub_for_library(&buffer_path, &parsed));
    let (entry_path, entry_text) = match &library {
        Some((stub_path, stub_text)) => (stub_path.clone(), stub_text.clone()),
        None => (buffer_path.clone(), source.to_string()),
    };

    let (lib_root, fallback_roots) = roots(&buffer_path, opts);
    let load_opts = LoadOptions {
        lib_root,
        fallback_roots,
        version,
        mode: LoadMode::Legacy,
        sources: Some(Box::new(BufferSources {
            entry: entry_path.clone(),
            text: entry_text,
        })),
    };
    let mut program = match rustyfi_loader::load(&entry_path, &load_opts) {
        Ok(program) => program,
        // An uninstalled package, no library root, a document opened outside
        // its project: not the user's mistake to answer for.
        Err(e) => return Analysis::degraded(version, format!("cannot resolve the program: {e}")),
    };

    if library.is_some() {
        let cst = into_loaded(parsed);
        match program.files.iter().position(|f| f.path == buffer_path) {
            // The graph already contains this file: a library one of its own
            // dependencies `@require:`s back. The loader read that copy off
            // the disk (the overlay only stands in for the stub), so swap in
            // the buffer's text rather than adding a second copy of every
            // binding.
            Some(i) => {
                program.files[i].cst = cst;
                program.files[i].version = version;
            }
            // The ordinary case: the buffer's own bindings, dependency-first —
            // after everything the library's headers pulled in, and before the
            // stub, which the loader always yields last.
            None => {
                let at = program.files.len() - 1;
                program.files.insert(
                    at,
                    LoadedFile {
                        path: buffer_path,
                        cst,
                        origin: FileOrigin::Local,
                        version,
                    },
                );
            }
        }
    }

    let files = program.files.len();
    match rustyfi_lang::check_document_program(program, version) {
        Ok(()) => Analysis {
            version,
            diagnostics: Vec::new(),
            depth: Depth::Program,
            note: None,
            files,
        },
        Err(e) => Analysis {
            version,
            diagnostics: vec![diagnose(&e, source)],
            depth: Depth::Program,
            note: None,
            files,
        },
    }
}

/// The library-root search path for a document at `entry`: the named roots if
/// there are any, else whatever discovery finds, split into the loader's
/// `(lib_root, fallback_roots)` pair.
fn roots(entry: &Path, opts: &CheckOptions) -> (Option<PathBuf>, Vec<PathBuf>) {
    let chain: Vec<PathBuf> = if !opts.lib_roots.is_empty() {
        opts.lib_roots.clone()
    } else {
        match opts.discover_roots {
            Some(discover) => discover(entry.parent().unwrap_or(Path::new("."))),
            None => Vec::new(),
        }
    };
    let mut chain = chain.into_iter();
    (chain.next(), chain.collect())
}

/// `parsed`, as the loader's own tagged CST.
fn into_loaded(parsed: Parsed) -> LoadedCst {
    match parsed {
        Parsed::V0_0(file) => LoadedCst::V0_0(file),
        Parsed::V0_1(file) => LoadedCst::V0_1(file),
        Parsed::None => unreachable!("an empty buffer never reaches the program tier"),
    }
}

/// A `(path, text)` pair for the stub document a library buffer is checked
/// underneath: a path that does not exist, in the library's own directory (so
/// `@import:` resolves identically), and a body of `()` behind the library's
/// own headers.
///
/// The headers are re-spelled from the parse rather than sliced out of the
/// source text, so the stub cannot accidentally carry part of a following
/// construct and does not depend on the buffer's layout.
fn stub_for_library(buffer: &Path, parsed: &Parsed) -> (PathBuf, String) {
    let mut text = String::new();
    match parsed {
        Parsed::V0_0(file) => {
            for header in &file.headers {
                push_header(&mut text, header);
            }
        }
        Parsed::V0_1(file) => {
            let headers = match file {
                rustyfi_syntax::cst_v1::FileV1::Document { headers, .. }
                | rustyfi_syntax::cst_v1::FileV1::Library { headers, .. } => headers,
            };
            for header in headers {
                if let rustyfi_syntax::cst_v1::HeaderV1::Legacy(h) = header {
                    push_header(&mut text, h);
                }
            }
        }
        Parsed::None => {}
    }
    // `()` is a body in both grammars. Nothing evaluates it — the check stops
    // after typechecking — so its type never has to be `document`.
    text.push_str("()\n");

    let name = buffer
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "buffer".to_string());
    let dir = buffer.parent().unwrap_or(Path::new("."));
    (dir.join(format!(".{name}.rustyfi-lsp-entry.saty")), text)
}

/// Re-spell one Legacy header onto the stub. `@stage:` is dropped on purpose:
/// it is a property of the *library's* bindings, the merge reads it off the
/// library's own file, and repeating it here would also stage the stub's `()`.
fn push_header(out: &mut String, header: &rustyfi_syntax::cst::Header) {
    use rustyfi_syntax::cst::Header;
    match header {
        Header::Require(tok) => out.push_str(&format!("@require: {}\n", tok.content)),
        Header::Import(tok) => out.push_str(&format!("@import: {}\n", tok.content)),
        Header::Stage(_) => {}
    }
}

/// The loader's filesystem, with one path served from memory.
///
/// The entry is the only overridden path; everything a `@require:`/`@import:`
/// reaches comes off the disk, which is what the compiler would do. Serving
/// several dirty buffers at once would need a document store and per-buffer
/// invalidation, and this seam is where that would go.
struct BufferSources {
    /// The entry's identity, as [`normalize`] computes it — which is also what
    /// [`SourceProvider::canonicalize`] hands back for it, so the loader's
    /// graph keys agree with this comparison.
    entry: PathBuf,
    /// The buffer's text, or the synthetic stub's.
    text: String,
}

impl BufferSources {
    fn is_entry(&self, path: &Path) -> bool {
        path == self.entry || normalize(path) == self.entry
    }
}

impl SourceProvider for BufferSources {
    fn read(&self, path: &Path) -> std::io::Result<String> {
        if self.is_entry(path) {
            return Ok(self.text.clone());
        }
        std::fs::read_to_string(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        // The entry exists even when the disk disagrees: an unsaved buffer, or
        // the stub document, which is never written anywhere.
        self.is_entry(path) || path.is_file()
    }

    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        if self.is_entry(path) {
            return Ok(self.entry.clone());
        }
        std::fs::canonicalize(path)
    }
}

/// The filesystem path a `file:` URI names, or `None` for any other scheme.
///
/// The rest of the crate treats a document URI as an opaque key and does not
/// depend on `url`; this is the one place a path is needed.
///
/// - `file:///a/b.saty` and `file://localhost/a/b.saty` both name
///   `/a/b.saty`; any other authority is a remote file this server cannot
///   read, so it declines rather than guess.
/// - `%XX` escapes are decoded byte-wise and the result must be UTF-8. The
///   Japanese corpus makes non-ASCII path components ordinary here.
/// - A Windows drive letter (`file:///C:/x`) loses the leading slash.
///
/// `untitled:` and an editor's private schemes return `None`: they name no
/// file, and the whole-program tier has nothing to resolve against.
pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Every LSP client sends the three-slash form; a non-empty, non-localhost
    // authority is a file on another machine.
    let path = match rest.find('/') {
        Some(0) => rest,
        _ => {
            let (authority, path) = rest.split_once('/')?;
            if !authority.is_empty() && authority != "localhost" {
                return None;
            }
            // `split_once` ate the separator.
            return decode(&format!("/{path}"));
        }
    };
    decode(path)
}

/// Percent-decode a URI path into a filesystem path.
fn decode(path: &str) -> Option<PathBuf> {
    let bytes = path.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8(out).ok()?;
    // `file:///C:/x` — the drive letter is the root, and the leading slash is
    // the URI's, not the path's.
    let trimmed = decoded.strip_prefix('/').unwrap_or(&decoded);
    let is_drive = {
        let mut c = trimmed.chars();
        matches!((c.next(), c.next()), (Some(a), Some(':')) if a.is_ascii_alphabetic())
    };
    Some(PathBuf::from(if is_drive { trimmed } else { &decoded }))
}

/// A path's identity for [`BufferSources`]: `std::fs::canonicalize` where the
/// file exists, and where it does not — an unsaved buffer, the library stub —
/// the canonical *directory* with the file name appended, so a project reached
/// through a symlink still compares equal.
fn normalize(path: &Path) -> PathBuf {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return canon;
    }
    if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
        if !dir.as_os_str().is_empty() {
            if let Ok(canon) = std::fs::canonicalize(dir) {
                return canon.join(name);
            }
        }
    }
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Errors → diagnostics
// ---------------------------------------------------------------------------

/// Turn a whole-program failure into one diagnostic against `source`.
fn diagnose(err: &rustyfi_lang::CompileError, source: &str) -> Diag {
    let (span, message) = located(err);
    let index = LineIndex::new(source);
    if let Some(span) = span.filter(|s| belongs_to_buffer(source, *s)) {
        let (start, end) = analysis::span_to_range(&index, span);
        return Diag {
            line: start.line,
            character: start.character,
            end_line: end.line,
            end_character: end.character,
            severity: Severity::Error,
            message,
        };
    }
    // No usable position *in this buffer*, but the error is real, so it is
    // reported at the top of the file rather than at a line the user did not
    // write. The two reasons are kept apart: "in another file" means some
    // dependency is at fault and the buffer may be perfect, while "no
    // position" means `typecheck.rs`'s `ast_span` had nothing to recover a
    // span from (it only handles `Var`/`Overwrite`/`AccessField`, so `1 + `x``
    // has nowhere to point). Conflating them sends the user hunting through
    // their packages for an error in the line they just typed.
    let end = index.position(first_char_end(source));
    let why = match span {
        Some(_) => "the position it carries belongs to another file of the program",
        None => "the compiler recorded no position for it",
    };
    Diag {
        line: 0,
        character: 0,
        end_line: end.line,
        end_character: end.character,
        severity: Severity::Error,
        message: format!("{message} (reported against the whole program: {why})"),
    }
}

/// The end of the buffer's first character, so the fallback range is
/// non-degenerate and the editor has something to draw.
fn first_char_end(source: &str) -> usize {
    source.chars().next().map(char::len_utf8).unwrap_or(0)
}

/// A failure's best span and its message, without the `Display` position
/// prefix — the range carries the position, and for an unattributable span
/// repeating it in words would be a lie.
fn located(err: &rustyfi_lang::CompileError) -> (Option<Span>, String) {
    use rustyfi_lang::CompileError as E;
    match err {
        E::Elaborate(e) => (Some(e.span), e.msg.clone()),
        E::Type(e) => {
            let message = match &e.source {
                Some(source) => format!("{}: {source}", e.message),
                None => e.message.clone(),
            };
            (e.span, message)
        }
        E::Lower(e) => (
            Some(e.span),
            format!("unsupported 0.1 construct: {} ({})", e.construct, e.hint),
        ),
        // `Parse` here is a *dependency*'s parse error — the buffer's own
        // parse already succeeded — and the rest carry no span at all.
        other => (None, other.to_string()),
    }
}

/// Whether `span` is an offset into `source` rather than into one of the
/// dependency files merged alongside it.
///
/// A `Span` is a `(line, col, byte)` triple per endpoint and carries no file
/// identity, so the triple is used as its own checksum: a foreign span passes
/// only if its byte offset falls on the same line at the same column here as
/// it did there, at *both* endpoints. Failure is safe — a rejected span is
/// reported at the top of the file with the reason attached, rather than at a
/// confidently wrong place.
///
/// A `line` of 0 is `Span::default()`, the marker for a synthesized node with
/// no source position (real lines are 1-based), and the same test rejects it.
fn belongs_to_buffer(source: &str, span: Span) -> bool {
    loc_matches(source, span.start) && loc_matches(source, span.end)
}

/// Whether `loc`'s line and column are what `source` says its byte offset's
/// line and column are.
fn loc_matches(source: &str, loc: Loc) -> bool {
    if loc.line == 0 || loc.byte > source.len() || !source.is_char_boundary(loc.byte) {
        return false;
    }
    line_col(source, loc.byte) == (loc.line, loc.col)
}

/// The lexer's own `(line, col)` for a byte offset: 1-based lines, 0-based
/// `char` columns, `\r\n` counted once — `rustyfi_syntax`'s `Lexer::bump`,
/// transcribed. It must be `bump`'s rule and not [`LineIndex`]'s, because this
/// compares against numbers `bump` produced.
///
/// The walk runs over the WHOLE source and stops at `byte`, never over the
/// `..byte` prefix: a `\r` at the end of the prefix has a `\n` after it in the
/// file and the lexer can see that, so deciding from the prefix alone counts
/// CRLF as a terminator there and lands every comparison on the wrong line.
fn line_col(source: &str, byte: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 0u32;
    let mut chars = source.char_indices().peekable();
    while let Some(&(at, c)) = chars.peek() {
        if at >= byte {
            break;
        }
        chars.next();
        let next = chars.peek().map(|&(_, c)| c);
        if c == '\n' || (c == '\r' && next != Some('\n')) {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_follows_the_lexers_own_counting() {
        assert_eq!(line_col("abc", 0), (1, 0));
        assert_eq!(line_col("abc", 2), (1, 2));
        assert_eq!(line_col("ab\ncd", 3), (2, 0));
        // CRLF is one terminator, and the `\r` still occupies a column.
        assert_eq!(line_col("ab\r\ncd", 3), (1, 3));
        assert_eq!(line_col("ab\r\ncd", 4), (2, 0));
        // A lone `\r` terminates a line.
        assert_eq!(line_col("ab\rcd", 3), (2, 0));
        // Columns are chars, not bytes.
        assert_eq!(line_col("あい", 6), (1, 2));
    }

    #[test]
    fn file_uris_decode_and_everything_else_declines() {
        let p = |s: &str| path_from_uri(s).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(p("file:///a/b.saty").as_deref(), Some("/a/b.saty"));
        assert_eq!(p("file://localhost/a/b.saty").as_deref(), Some("/a/b.saty"));
        assert_eq!(p("file:///a/my%20doc.saty").as_deref(), Some("/a/my doc.saty"));
        // The corpus is full of Japanese, and so are its filenames.
        assert_eq!(
            p("file:///a/%E6%96%87%E6%9B%B8.saty").as_deref(),
            Some("/a/文書.saty")
        );
        assert_eq!(p("file:///C:/a/b.saty").as_deref(), Some("C:/a/b.saty"));
        // No path to analyse against.
        assert_eq!(p("untitled:Untitled-1"), None);
        assert_eq!(p("file://otherhost/a/b.saty"), None);
        assert_eq!(p("https://example.com/a.saty"), None);
    }

    #[test]
    fn a_span_from_another_file_is_not_claimed_as_this_ones() {
        let source = "let x = 1\nlet y = 2\n";
        let loc = |line, col, byte| Loc { line, col, byte };
        let here = Span {
            start: loc(2, 4, 14),
            end: loc(2, 5, 15),
        };
        assert!(belongs_to_buffer(source, here));
        // The same byte offsets, but a different file's line/col layout.
        let elsewhere = Span {
            start: loc(9, 4, 14),
            end: loc(9, 5, 15),
        };
        assert!(!belongs_to_buffer(source, elsewhere));
        // The synthesized-node marker.
        assert!(!belongs_to_buffer(source, Span::default()));
        // Past the end of the buffer.
        let beyond = Span {
            start: loc(1, 0, 900),
            end: loc(1, 1, 901),
        };
        assert!(!belongs_to_buffer(source, beyond));
    }
}
