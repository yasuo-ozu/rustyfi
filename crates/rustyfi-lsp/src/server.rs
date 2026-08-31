//! The stdio language server: lifecycle, document sync, and diagnostics.
//!
//! Everything the server *knows* is somewhere else — [`crate::analyze`] and
//! [`crate::project::check`] for diagnostics, [`crate::build_model`] for the
//! cursor-driven requests, [`crate::document_symbols`] for the outline. This
//! module only decides which of them to ask, and how to phrase the answer.
//!
//! # What is implemented
//!
//! `initialize`, `initialized`, `shutdown`, `exit`,
//! `textDocument/didOpen` / `didChange` / `didClose`,
//! `textDocument/publishDiagnostics`, the three interactive requests —
//! `textDocument/hover`, `textDocument/definition` and
//! `textDocument/completion` — the two outline ones,
//! `textDocument/documentSymbol` and `workspace/symbol`, and
//! `textDocument/formatting`. The `initialize`
//! reply advertises exactly that and nothing more — an over-claimed
//! capability costs the user a hang or an empty popup on every keystroke, so
//! the reply lists only what is actually wired up. (The whole-program tier is
//! the one thing here that is not a capability at all: it changes what a
//! diagnostic can be *about*, not which methods exist, so a client that spoke
//! to the parse-only server speaks to this one unchanged.)
//!
//! Two of those touch the filesystem and the rest answer purely from the text
//! the client sent: `workspace/symbol` reads the project's other files — see
//! the crate-private `workspace` module, which owns all of that — and
//! `textDocument/definition` follows a `@require:`/`@import:` header through
//! the compiler's own loader (`State::resolve_header`).
//!
//! Document sync is **full**, not incremental. Incremental sync would mean
//! reimplementing UTF-16-range splicing over the buffer, and the whole
//! analysis re-parses the file anyway (this port's parser has no incremental
//! mode), so the only thing incremental sync could save is the bytes on the
//! wire. Full sync is the honest choice here and is advertised as such.
//!
//! # Why there is now a document store
//!
//! Diagnostics are *pushed*: the notification that changes a buffer carries
//! the buffer, so nothing had to be remembered between messages, and not
//! remembering is how staleness bugs are avoided. Every other request is
//! *pulled* — `textDocument/hover` carries a URI and a position and no text
//! at all, `textDocument/documentSymbol` a URI and nothing else — so the
//! server has to hold what the client last sent it. `State::docs` is that
//! and only that: it is written on `didOpen`/`didChange`, dropped on
//! `didClose`, and never derived from. The alternative — reading the URI back
//! off disk — would answer about the saved file rather than the buffer being
//! edited, and would put filesystem access in the pure half of this module.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::jsonrpc::{self, code, Incoming};
use crate::model::HeaderKind;
use crate::workspace::Workspace;
use crate::{ByteRange, Definition, LineIndex, Position, RustyfiVersion, Symbol};

/// How the server was started.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// `rustyfi lsp --lang 0.1`: force every buffer to one generation.
    ///
    /// `None` — the default — detects per file (see [`crate::analyze_auto`],
    /// which is the CLI's own entry-document rule plus a re-check for buffers
    /// that signal no version at all). An override is worth having for a
    /// project that is wholly one generation and whose library files, being
    /// signal-free, would otherwise each be parsed twice.
    pub lang: Option<RustyfiVersion>,

    /// Where `@require:` looks, for go-to-definition on a header.
    ///
    /// The same root `rustyfi --lib-root` names, and resolved by the same
    /// function the compiler uses (`rustyfi_loader::resolve_require`), so the
    /// editor cannot disagree with the build about which file a header names.
    /// `None` means no root is configured and a `@require:` simply does not
    /// resolve — `@import:`, which is relative to the file itself, still does.
    ///
    /// Separate from the copy inside [`Self::project`] because the two are
    /// independently optional: `--no-typecheck` leaves the whole-program tier
    /// off and a header still has to be followable. `rustyfi lsp` fills both
    /// in from the one `--lib-root`.
    pub lib_root: Option<PathBuf>,

    /// Whole-program analysis: resolve each buffer's `@require:`/`@import:`
    /// graph and typecheck it, not just parse it
    /// ([`crate::project::check`]).
    ///
    /// `None` — the default — is the parse tier alone, which is what a
    /// consumer with no filesystem to offer (and every test in
    /// `tests/server_stdio.rs`) wants. `rustyfi lsp` sets `Some(..)` unless
    /// `--no-typecheck` is given, and fills in root discovery, which it can
    /// do and this crate deliberately cannot (see
    /// [`crate::project::CheckOptions::discover_roots`]).
    ///
    /// [`Self::lang`] wins over the copy inside it, so the two cannot say
    /// different things about the same buffer.
    #[cfg(feature = "typecheck")]
    pub project: Option<crate::project::CheckOptions>,
}

/// Run the server against a pair of streams until the client disconnects or
/// sends `exit`, returning the process exit code.
///
/// Streams rather than `stdin()`/`stdout()` so the whole protocol can be
/// driven in-process by a test with two byte buffers — which is what
/// `tests/server_stdio.rs` does, so the end-to-end path under test is the
/// same code path the binary runs, not a re-implementation of it.
///
/// The exit code follows the specification's `exit` rules: `0` if `shutdown`
/// was received first, `1` otherwise. A client that simply closes the pipe
/// without either is treated as a clean disconnect (`0`) — that is the normal
/// way an editor goes away when it crashes or is killed, and there is nothing
/// to report to.
pub fn run(input: &mut impl BufRead, output: &mut impl Write, opts: Options) -> io::Result<i32> {
    let mut state = State::new(opts);
    loop {
        let msg = match jsonrpc::read_message(input) {
            Ok(Some(msg)) => msg,
            Ok(None) => return Ok(0),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                // Malformed JSON: answer with a null-id parse error (the only
                // response JSON-RPC allows when the id is unknowable) and
                // keep the session alive.
                jsonrpc::write_message(
                    output,
                    &jsonrpc::error_response(Value::Null, code::PARSE_ERROR, &e.to_string()),
                )?;
                continue;
            }
            Err(e) => return Err(e),
        };

        match jsonrpc::classify(msg) {
            Incoming::Response => {}
            Incoming::Request { id, method, params } => {
                match state.request(&method, params) {
                    Ok(result) => jsonrpc::write_message(output, &jsonrpc::response(id, result))?,
                    Err(err) => jsonrpc::write_message(
                        output,
                        &jsonrpc::error_response(id, err.0, &err.1),
                    )?,
                }
            }
            Incoming::Notification { method, params } => {
                if method == "exit" {
                    return Ok(if state.shutdown_requested { 0 } else { 1 });
                }
                for out in state.notification(&method, params) {
                    jsonrpc::write_message(output, &out)?;
                }
            }
        }
    }
}

/// `(code, message)` for an error response.
type RequestError = (i64, String);

/// Everything the server remembers between messages.
///
/// Including the open buffers, which a diagnostics-only server did not need —
/// see the module comment. The store is keyed by the URI as an **opaque
/// string**; a path is derived from it only where one is genuinely needed
/// (the whole-program tier, to resolve `@import:` and discover a library root
/// — [`crate::project::path_from_uri`] — and go-to-definition on a header),
/// and a URI with no path simply falls back to the parse tier. Entries live
/// from `didOpen` to `didClose`, which is exactly the window the
/// specification says a server may be asked about a document in.
struct State {
    opts: Options,
    initialized: bool,
    shutdown_requested: bool,
    /// The text of each open buffer, keyed by URI. Replaced wholesale on
    /// every `didChange` (sync is Full), so it cannot go stale against what
    /// the client has.
    ///
    /// Nothing derived is cached beside it: a [`crate::Model`] is rebuilt per
    /// request, because a cache keyed on a buffer that changes on every
    /// keystroke is a staleness bug waiting for a race, and the parse is
    /// budgeted (`budget`) so its cost is bounded.
    docs: HashMap<String, String>,
    /// The project's folders and their cached outlines, for
    /// `workspace/symbol`. Unlike [`Self::docs`] this one *does* read the
    /// filesystem — see the `workspace` module, which owns all of it.
    workspace: Workspace,
}

impl State {
    fn new(opts: Options) -> Self {
        State {
            opts,
            initialized: false,
            shutdown_requested: false,
            docs: HashMap::new(),
            workspace: Workspace::default(),
        }
    }

    /// Handle a request, returning its `result` or an error to report.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, RequestError> {
        if self.shutdown_requested {
            return Err((
                code::INVALID_REQUEST,
                format!("{method} arrived after shutdown"),
            ));
        }
        match method {
            "initialize" => {
                self.absorb_initialization_options(&params);
                self.workspace.absorb_initialize(&params);
                self.initialized = true;
                Ok(server_capabilities())
            }
            _ if !self.initialized => Err((
                code::SERVER_NOT_INITIALIZED,
                format!("{method} arrived before initialize"),
            )),
            "shutdown" => {
                self.shutdown_requested = true;
                Ok(Value::Null)
            }
            // The three cursor-driven requests. Each answers `null` — never an
            // error — when it has nothing to say: LSP treats `null` as "no
            // result", and an error response makes a client log a failure for
            // what is an ordinary outcome (a cursor on a keyword, a name from
            // a package this buffer cannot see).
            "textDocument/hover" => Ok(self.hover(&params)),
            "textDocument/definition" => Ok(self.definition(&params)),
            "textDocument/completion" => Ok(self.completion(&params)),
            // The two outline requests, on the same principle but with an
            // empty *list* as their "nothing to say" — a pane, unlike a popup,
            // is always on screen, and an error in it reads as a broken server
            // rather than as an empty file.
            "textDocument/documentSymbol" => Ok(self.document_symbols(&params)),
            "textDocument/formatting" => Ok(self.formatting(&params)),
            "workspace/symbol" => {
                let query = params.get("query").and_then(Value::as_str).unwrap_or("");
                let lang = self.opts.lang;
                Ok(Value::Array(self.workspace.query(query, &self.docs, lang)))
            }
            _ => Err((code::METHOD_NOT_FOUND, format!("{method} is not supported"))),
        }
    }

    /// The buffer a request is about, and the byte offset its position names.
    ///
    /// `None` when the client asks about a document it never sent — which
    /// happens legitimately, when a request crosses a `didClose` on the wire.
    fn locate<'a>(&'a self, params: &Value) -> Option<(&'a str, usize)> {
        let doc = params.get("textDocument")?;
        let text = self.docs.get(str_field(doc, "uri")?)?;
        let pos = params.get("position")?;
        let position = Position {
            line: pos.get("line")?.as_u64()? as u32,
            character: pos.get("character")?.as_u64()? as u32,
        };
        Some((text, LineIndex::new(text).offset(position)))
    }

    fn hover(&self, params: &Value) -> Value {
        let Some((text, byte)) = self.locate(params) else {
            return Value::Null;
        };
        let model = crate::build_model(text, self.opts.lang);
        match crate::hover(&model, byte) {
            None => Value::Null,
            Some(h) => json!({
                "contents": { "kind": "markdown", "value": h.markdown },
                "range": lsp_range(text, h.range),
            }),
        }
    }

    fn definition(&self, params: &Value) -> Value {
        let (Some(uri), Some((text, byte))) = (
            params.get("textDocument").and_then(|d| str_field(d, "uri")),
            self.locate(params),
        ) else {
            return Value::Null;
        };
        let model = crate::build_model(text, self.opts.lang);
        match crate::definition(&model, byte) {
            Some(Definition::Here(range)) => json!({
                "uri": uri,
                "range": lsp_range(text, range),
            }),
            Some(Definition::OtherFile { kind, name }) => match self.resolve_header(uri, kind, &name)
            {
                // The whole file, from its origin: a client that jumps here
                // opens it at the top, which is where a package's own
                // documentation and its `module` head are.
                Some(path) => json!({
                    "uri": path_to_uri(&path),
                    "range": { "start": { "line": 0, "character": 0 },
                               "end": { "line": 0, "character": 0 } },
                }),
                None => Value::Null,
            },
            None => Value::Null,
        }
    }

    /// Turn a `@require:`/`@import:` name into a path, exactly the way the
    /// compiler's loader does.
    ///
    /// `@import:` needs nothing configured — it is relative to the file that
    /// wrote the header — so it resolves in any project. `@require:` needs a
    /// library root, and without one this answers nothing rather than
    /// searching somewhere plausible.
    fn resolve_header(&self, uri: &str, kind: HeaderKind, name: &str) -> Option<PathBuf> {
        let here = uri_to_path(uri)?;
        let sources = rustyfi_loader::FsSources;
        match kind {
            HeaderKind::Import => {
                rustyfi_loader::resolve_import(&sources, here.parent()?, name).ok()
            }
            HeaderKind::Require => {
                let root = self.opts.lib_root.as_deref()?;
                let version = self.opts.lang.unwrap_or(RustyfiVersion::DEFAULT);
                rustyfi_loader::resolve_require(&sources, &[root], name, version).ok()
            }
            // 0.1's `use package` is resolved through an envelope graph, not a
            // search path; guessing at a file for it would be a guess.
            HeaderKind::Use => None,
        }
    }

    fn completion(&self, params: &Value) -> Value {
        let Some((text, byte)) = self.locate(params) else {
            return Value::Null;
        };
        let model = crate::build_model(text, self.opts.lang);
        let items: Vec<Value> = crate::completions(&model, byte)
            .into_iter()
            .map(|c| {
                json!({
                    "label": c.label,
                    "kind": c.kind,
                    "detail": c.detail,
                    // A `textEdit` rather than bare insertion: the word being
                    // replaced starts before the cursor (and, for a command,
                    // before its `\`), and a client left to guess the replaced
                    // range from its own word pattern would leave the sigil
                    // behind.
                    "textEdit": {
                        "range": lsp_range(text, c.range),
                        "newText": c.label,
                    },
                })
            })
            .collect();
        json!({ "isIncomplete": false, "items": items })
    }

    /// Handle a notification, returning any messages to send as a result.
    ///
    /// `exit` is handled by the loop, not here, because it ends the process
    /// rather than producing a message.
    fn notification(&mut self, method: &str, params: Value) -> Vec<Value> {
        // Notifications before `initialize` "should be dropped" per the
        // specification, except `exit` (handled by the caller). The same
        // applies once `shutdown` has been answered: the session is winding
        // down, and pushing diagnostics at a client that has stopped
        // listening is at best noise.
        if !self.initialized || self.shutdown_requested {
            return Vec::new();
        }
        match method {
            // The two carry the buffer differently — `didOpen` inside the
            // `textDocument` object, `didChange` in `contentChanges` — and
            // are otherwise the same notification.
            "textDocument/didOpen" => {
                let text = params
                    .get("textDocument")
                    .and_then(|d| str_field(d, "text"))
                    .map(str::to_string);
                self.remember(&params, text.as_deref());
                self.publish(&params, text.as_deref())
            }
            // `full_replacement` yields `None` for a *ranged* change under a
            // Full-sync agreement: the client is not honouring the advertised
            // capability, and applying it would need the incremental splicing
            // this server does not do. Publishing nothing leaves the previous
            // diagnostics on screen — stale, but never pointing at text that
            // is not there.
            "textDocument/didChange" => match full_replacement(&params) {
                Change::Full(text) => {
                    let text = text.to_string();
                    self.remember(&params, Some(&text));
                    self.publish(&params, Some(&text))
                }
                // Nothing changed, so nothing is stale: keep the buffer, and
                // send no diagnostics — the ones on screen still describe it.
                Change::Nothing => Vec::new(),
                Change::Unreadable => {
                    self.remember(&params, None);
                    self.publish(&params, None)
                }
            },
            "textDocument/didClose" => {
                let Some(uri) = params
                    .get("textDocument")
                    .and_then(|d| str_field(d, "uri"))
                    .map(str::to_string)
                else {
                    return Vec::new();
                };
                self.docs.remove(&uri);
                // An empty list is how a server retracts diagnostics; without
                // it the editor keeps showing them for a file that is gone.
                vec![jsonrpc::notification(
                    "textDocument/publishDiagnostics",
                    json!({ "uri": uri, "diagnostics": [] }),
                )]
            }
            // `initialized`, `$/cancelRequest`, `$/setTrace`, and anything
            // else: silently ignored, which is what the specification asks of
            // a server for an unknown notification.
            _ => Vec::new(),
        }
    }

    /// Store (or replace) what the client just sent, so a later
    /// `hover`/`definition`/`completion`/`documentSymbol` — which carry a URI
    /// and no text — has a buffer to answer about.
    ///
    /// A notification with no text **forgets** the buffer rather than keeping
    /// the previous one. That is a ranged `didChange` under a Full-sync
    /// agreement — the client is not honouring the advertised capability — and
    /// the text the server holds is then known to be out of date. Diagnostics
    /// may stay on screen through such a change (they are at worst
    /// mispositioned), but a hover computed from stale text *answers about
    /// characters that are not there*, a jump computed from it lands somewhere
    /// the user did not ask for, and a stale outline is harder to notice than
    /// a missing one. Answering nothing is the only honest option.
    fn remember(&mut self, params: &Value, text: Option<&str>) {
        let Some(uri) = params
            .get("textDocument")
            .and_then(|d| str_field(d, "uri"))
            .map(str::to_string)
        else {
            return;
        };
        match text {
            Some(text) => self.docs.insert(uri, text.to_string()),
            None => self.docs.remove(&uri),
        };
    }

    /// Publish diagnostics for `text` against the URI and version in
    /// `params.textDocument`, or nothing if either the URI or the text is
    /// missing.
    ///
    /// `text` is a parameter rather than read from `params` because that is
    /// the *only* thing `didOpen` and `didChange` disagree about; everything
    /// else — the URI, the version echo, the "drop a malformed notification
    /// silently" rule — is shared, and was previously written twice.
    fn publish(&self, params: &Value, text: Option<&str>) -> Vec<Value> {
        let Some(doc) = params.get("textDocument") else {
            return Vec::new();
        };
        let (Some(uri), Some(text)) = (str_field(doc, "uri"), text) else {
            return Vec::new();
        };
        vec![self.diagnostics_for(uri, text, doc.get("version"))]
    }

    /// `textDocument/documentSymbol`: the outline of one open buffer.
    ///
    /// A URI this server has never been given is answered with an empty list
    /// rather than an error. The specification lets a server return `null`,
    /// but an editor showing "request failed" in its outline pane for a file
    /// it simply has not opened yet is noise; an empty outline says the same
    /// thing quietly.
    fn document_symbols(&self, params: &Value) -> Value {
        let text = params
            .get("textDocument")
            .and_then(|d| str_field(d, "uri"))
            .and_then(|uri| self.docs.get(uri));
        let Some(text) = text else {
            return Value::Array(Vec::new());
        };
        let symbols = match self.opts.lang {
            Some(lang) => crate::document_symbols(text, lang),
            None => crate::document_symbols_auto(text),
        };
        Value::Array(symbols.iter().map(symbol_json).collect())
    }

    /// `textDocument/formatting`: normalise the buffer's program-area
    /// whitespace.
    ///
    /// Three outcomes, and the difference between the last two is the whole
    /// contract:
    ///
    /// - **`null`** — the formatter *declined*. A URI this server was never
    ///   sent, or a buffer that does not lex ([`crate::format`] explains why
    ///   that is where it stops). `null` is a specified result for this
    ///   request, not an error, so a client shows "cannot format" rather than
    ///   logging a failure.
    /// - **`[]`** — the buffer is already formatted. Distinct from `null` on
    ///   purpose: nothing is wrong, there is simply nothing to do, and a
    ///   format-on-save that answered `null` here would tell the user their
    ///   file is unformattable every time they saved a tidy one.
    /// - **one `TextEdit`** — the change, narrowed to the bytes that actually
    ///   differ (see [`minimal_edit`]).
    fn formatting(&self, params: &Value) -> Value {
        let text = params
            .get("textDocument")
            .and_then(|d| str_field(d, "uri"))
            .and_then(|uri| self.docs.get(uri));
        let Some(text) = text else {
            return Value::Null;
        };
        let opts = format_options(params.get("options"));
        let formatted = match self.opts.lang {
            Some(lang) => crate::format(text, lang, &opts),
            None => crate::format_auto(text, &opts),
        };
        match formatted {
            None => Value::Null,
            Some(new) if new == *text => Value::Array(Vec::new()),
            Some(new) => Value::Array(vec![minimal_edit(text, &new)]),
        }
    }

    /// Absorb `initializationOptions` — `lang`, `libRoot`, `typecheck`,
    /// `checkLibraries` — for the settings a client can send that the command
    /// line may not have pinned.
    ///
    /// The command line wins on every one of them: it is the more explicit of
    /// the two, and an editor that guesses wrong in its client config should
    /// not be able to override what the user typed when launching the server.
    /// For the flag-shaped options that means "the command line only ever
    /// *disables*", which is why each is applied here only in the direction
    /// the flag cannot have chosen.
    fn absorb_initialization_options(&mut self, params: &Value) {
        let options = params.get("initializationOptions");
        if self.opts.lang.is_none() {
            if let Some(lang) = options
                .and_then(|o| o.get("lang"))
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<RustyfiVersion>().ok())
            {
                self.opts.lang = Some(lang);
            }
        }
        // `libRoot` follows the same precedence rule and for the same reason,
        // and falls back to the environment variable the CLI already honours
        // so that an editor started from a configured shell needs no client
        // configuration at all. That fallback is also why this function does
        // not bail out when the client sent no `initializationOptions` at all.
        if self.opts.lib_root.is_none() {
            self.opts.lib_root = options
                .and_then(|o| o.get("libRoot"))
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("RUSTYFI_LIB_ROOT").map(PathBuf::from));
        }
        #[cfg(feature = "typecheck")]
        {
            if let Some(o) = options {
                // `typecheck: false` turns the whole tier off; nothing here
                // can turn it ON, because the roots and the discovery hook
                // come from the process that started the server.
                if o.get("typecheck") == Some(&Value::Bool(false)) {
                    self.opts.project = None;
                }
                if let Some(project) = &mut self.opts.project {
                    if o.get("checkLibraries") == Some(&Value::Bool(true)) {
                        project.check_libraries = true;
                    }
                    // `libRoot` accepts a string or an array of them — the
                    // loader's search path is a list, and an editor
                    // configuring one project-local root plus a shared one
                    // should not have to choose. Named roots replace
                    // discovery entirely, exactly as `--lib-root` does for
                    // the compiler.
                    let roots: Vec<PathBuf> = match o.get("libRoot") {
                        Some(Value::String(s)) => vec![s.into()],
                        Some(Value::Array(items)) => items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(Into::into)
                            .collect(),
                        _ => Vec::new(),
                    };
                    if !roots.is_empty() && project.lib_roots.is_empty() {
                        project.lib_roots = roots;
                    }
                }
            }
        }
    }

    /// Analyse `text` and build the whole `textDocument/publishDiagnostics`
    /// notification, envelope included.
    fn diagnostics_for(&self, uri: &str, text: &str, version: Option<&Value>) -> Value {
        let items: Vec<Value> = self
            .diagnose(uri, text)
            .into_iter()
            .map(|d| {
                json!({
                    "range": {
                        "start": { "line": d.line, "character": d.character },
                        "end": { "line": d.end_line, "character": d.end_character },
                    },
                    "severity": d.severity.code(),
                    "source": "rustyfi",
                    "message": d.message,
                })
            })
            .collect();
        let mut params = json!({ "uri": uri, "diagnostics": items });
        // Echo the document version the diagnostics were computed from, so a
        // client can discard a result that a later edit has already
        // invalidated (LSP 3.17 `PublishDiagnosticsParams.version`).
        if let Some(v @ Value::Number(_)) = version {
            params["version"] = v.clone();
        }
        jsonrpc::notification("textDocument/publishDiagnostics", params)
    }

    /// The diagnostics themselves: the whole-program tier where it is
    /// configured and the URI names a path it can reach, the parse tier
    /// otherwise.
    ///
    /// A URI that is not a `file:` one — `untitled:`, an editor's own
    /// scratch scheme — has no path, so there is no directory to resolve
    /// `@import:` against and no library root to discover. That buffer gets
    /// the parse tier, which is exactly what it would have got before this
    /// tier existed.
    #[cfg(feature = "typecheck")]
    fn diagnose(&self, uri: &str, text: &str) -> Vec<crate::Diag> {
        let Some(project) = &self.opts.project else {
            return self.parse_only(text);
        };
        let Some(path) = crate::project::path_from_uri(uri) else {
            return self.parse_only(text);
        };
        let opts = crate::project::CheckOptions {
            // The command line's `--lang` wins over the copy the caller put
            // in `project`, so the two halves of one server cannot disagree
            // about which generation a buffer is.
            lang: self.opts.lang.or(project.lang),
            ..project.clone()
        };
        crate::project::check(&path, text, &opts).diagnostics
    }

    #[cfg(not(feature = "typecheck"))]
    fn diagnose(&self, _uri: &str, text: &str) -> Vec<crate::Diag> {
        self.parse_only(text)
    }

    /// Lex + parse only — [`crate::analyze`] under the configured generation.
    fn parse_only(&self, text: &str) -> Vec<crate::Diag> {
        match self.opts.lang {
            Some(lang) => crate::analyze(text, lang),
            None => crate::analyze_auto(text),
        }
    }
}

/// One [`Symbol`] as LSP's `DocumentSymbol`, children and all.
///
/// `detail` and `children` are omitted when they are empty — both are
/// optional in the protocol, and a thousand `"children": []` members is a
/// measurable fraction of the payload for a library with a big signature.
fn symbol_json(s: &Symbol) -> Value {
    let mut out = json!({
        "name": s.name,
        "kind": s.kind.code(),
        "range": {
            "start": { "line": s.range.start.line, "character": s.range.start.character },
            "end": { "line": s.range.end.line, "character": s.range.end.character },
        },
        "selectionRange": {
            "start": {
                "line": s.selection_range.start.line,
                "character": s.selection_range.start.character,
            },
            "end": {
                "line": s.selection_range.end.line,
                "character": s.selection_range.end.character,
            },
        },
    });
    if let Some(detail) = &s.detail {
        out["detail"] = Value::String(detail.clone());
    }
    if !s.children.is_empty() {
        out["children"] = Value::Array(s.children.iter().map(symbol_json).collect());
    }
    out
}

/// A [`ByteRange`] as an LSP range over `text`.
fn lsp_range(text: &str, range: ByteRange) -> Value {
    let index = LineIndex::new(text);
    let (start, end) = (index.position(range.start), index.position(range.end));
    json!({
        "start": { "line": start.line, "character": start.character },
        "end": { "line": end.line, "character": end.character },
    })
}

/// The largest `tabSize` this server will honour.
///
/// `tabSize` is a `uinteger` on the wire, so it is unbounded client input, and
/// `format::normalise_indent` turns it into that many spaces per indentation
/// run — a client-controlled allocation, on a request an editor sends on every
/// save. Measured: `tabSize: 40_000_000` on a file with one tab-indented line
/// produced a 120 MB `newText` in 0.45 s.
///
/// 256 is the bound because it is far past any editor's own tab stop (8 is the
/// widest anybody defaults to, and the settings UIs present single digits)
/// while still being wider than any line a person reads, so no setting a user
/// could plausibly have chosen is clipped; and because it keeps the worst case
/// proportional to the file — an indentation run of `n` tabs can grow to at
/// most `256n` bytes. `0` is clamped *up* to `1` for the reason
/// `normalise_indent` already had: a tab stop every zero columns names no
/// column.
/// One bound, not two: `crate::format` enforces the same ceiling for library
/// callers (the wasm playground among them), who never pass through this
/// function at all. Two constants for one field would be a difference nobody
/// could explain.
const MAX_TAB_SIZE: u64 = crate::format::MAX_TAB_SIZE as u64;

/// LSP's `FormattingOptions` object, as far as this formatter reads it.
///
/// **An absent optional member means "off", not "the library default".**
/// `tabSize` and `insertSpaces` are required by the specification;
/// `trimTrailingWhitespace`, `insertFinalNewline` and `trimFinalNewlines` are
/// optional, and the common clients send them *only when the user's
/// corresponding editor setting is on*. VS Code's
/// `files.trimTrailingWhitespace`, `files.insertFinalNewline` and
/// `files.trimFinalNewlines` all default to false, and the client then omits
/// the member entirely rather than sending `false`; nvim behaves the same. So
/// an ordinary format-on-save arrives as `{"tabSize":4,"insertSpaces":true}`,
/// and reading that as "all three on" deletes trailing whitespace and final
/// newlines the user explicitly turned off. Silence from a client is not a
/// request.
///
/// This is deliberately **not** [`crate::FormatOptions`]'s own [`Default`],
/// and the two must not be collapsed into one. `FormatOptions::default()` is
/// for the library and playground callers, who pass no options because there
/// is no client in the picture at all, and for whom "tidy everything" is the
/// right answer. Here the silence belongs to somebody, and it carries
/// information. Only the two *required* members fall back to the library
/// default, and that fallback is unreachable for a conforming client — it
/// exists so a malformed request still gets an answer instead of an error.
///
/// # The contract that results
///
/// Two of the formatter's rules are not LSP options, so they have no member to
/// be absent and do **not** switch off with these flags — they always apply:
///
/// - a run of blank lines in program text is capped at
///   [`crate::FormatOptions::max_blank_lines`];
/// - a file's *leading* blank lines are dropped.
///
/// That is not an inconsistency with "absence means off". The protocol has no
/// vocabulary for either rule, so a client cannot ask for them or decline them
/// either way; and a formatter that did nothing whatever when the three
/// optional members were missing would answer `[]` to every ordinary
/// format-on-save, which is an advertised capability that never does anything.
/// The visible consequence is at the end of a file: with `trimFinalNewlines`
/// off, a file ending in six newlines still comes back with that run capped,
/// so "off" means *this formatter does not collapse the tail to a single
/// newline*, not *the tail is untouched*.
///
/// `FormattingOptions` also carries arbitrary client-defined members, which
/// are ignored — acting on a key this server does not document would be acting
/// on a guess about what the client meant.
fn format_options(options: Option<&Value>) -> crate::FormatOptions {
    let default = crate::FormatOptions::default();
    // A missing `options` object is treated exactly as an empty one rather
    // than as the library default: the three optional members are absent
    // either way, and the two required ones fall back below.
    let get = |name: &str| options.and_then(|o| o.get(name));
    let flag = |name: &str| get(name).and_then(Value::as_bool).unwrap_or(false);
    crate::FormatOptions {
        tab_size: get("tabSize")
            .and_then(Value::as_u64)
            .map_or(default.tab_size, |n| n.clamp(1, MAX_TAB_SIZE) as usize),
        insert_spaces: get("insertSpaces")
            .and_then(Value::as_bool)
            .unwrap_or(default.insert_spaces),
        trim_trailing_whitespace: flag("trimTrailingWhitespace"),
        insert_final_newline: flag("insertFinalNewline"),
        trim_final_newlines: flag("trimFinalNewlines"),
        max_blank_lines: default.max_blank_lines,
    }
}

/// The one `TextEdit` that turns `old` into `new`, narrowed to the bytes that
/// differ.
///
/// A whole-document replacement would be correct and is what the simplest
/// server sends. It is also what makes an editor scroll to the top, collapse
/// every fold and lose the selection on every save, because from the client's
/// point of view every line was deleted and rewritten. Trimming the common
/// prefix and suffix costs two loops and leaves an edit that usually covers a
/// handful of lines.
///
/// Both cuts land on `char` boundaries, and both are boundaries in *both*
/// strings for the same reason: the bytes on either side of the cut are equal
/// in the two strings, and a byte position in valid UTF-8 is a boundary
/// exactly when the byte there is not a continuation byte.
fn minimal_edit(old: &str, new: &str) -> Value {
    let (start, old_end, new_end) = minimal_edit_span(old, new);
    json!({
        "range": lsp_range(old, ByteRange::new(start, old_end)),
        "newText": &new[start..new_end],
    })
}

/// [`minimal_edit`]'s arithmetic, as byte offsets: replace `old[start..
/// old_end]` with `new[start..new_end]`.
///
/// Split out from the JSON so the property that matters can be STATED —
/// `old[..start] + new[start..new_end] + old[old_end..] == new`. Against a
/// `Value` carrying a UTF-16 line/character range that round trip cannot be
/// written, which is why this function was the one link between the formatter
/// and the client's buffer that nothing exercised: a wrong cut here does not
/// produce tidy-but-odd whitespace, it silently deletes the user's text.
fn minimal_edit_span(old: &str, new: &str) -> (usize, usize, usize) {
    let (ob, nb) = (old.as_bytes(), new.as_bytes());
    let mut start = 0;
    while start < ob.len() && start < nb.len() && ob[start] == nb[start] {
        start += 1;
    }
    start = crate::line_index::floor_boundary(old, start);

    let mut back = 0;
    while back < ob.len() - start
        && back < nb.len() - start
        && ob[ob.len() - 1 - back] == nb[nb.len() - 1 - back]
    {
        back += 1;
    }
    // Round the cut *outwards* — a shorter suffix, a longer edit — because
    // rounding it inwards could put it inside a character.
    while back > 0 && !old.is_char_boundary(ob.len() - back) {
        back -= 1;
    }

    (start, ob.len() - back, nb.len() - back)
}

/// `file:///a/b%20c.saty` → `/a/b c.saty`.
///
/// Deliberately minimal, and `None` for anything that is not a plain local
/// `file:` URI: the only thing a path is used for here is following a
/// `@require:`/`@import:` header, and a scheme this does not understand is one
/// where guessing at a path would open the wrong file — or none.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file://host/path` names another machine; only an empty authority (or
    // `localhost`) is this filesystem.
    let path = match rest.strip_prefix("localhost") {
        Some(p) => p,
        None => rest,
    };
    if !path.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(path)?;
    // `file:///C:/x` — a Windows drive letter arrives with a leading slash
    // that is part of the URI grammar and not of the path.
    let bytes = decoded.as_bytes();
    let trimmed = match bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':' {
        true => &decoded[1..],
        false => &decoded[..],
    };
    Some(PathBuf::from(trimmed))
}

/// The inverse, escaping the characters a URI may not carry literally.
fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::from("file://");
    // A Windows path has no leading slash of its own.
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decode `%XX` escapes. `None` for a malformed escape, which is a URI this
/// server should not be inventing a path from.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The `initialize` result.
fn server_capabilities() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": {
                "openClose": true,
                // 1 = Full. See the module comment for why not Incremental.
                "change": 1,
            },
            "hoverProvider": true,
            "definitionProvider": true,
            "completionProvider": {
                // The sigils that decide a namespace, plus `.` for a module
                // member. A client that only auto-triggers on these gets the
                // cases this server is confident about and nothing else; one
                // that also triggers on word characters still gets a sensible
                // answer, because a bare word in prose completes to nothing.
                "triggerCharacters": ["\\", "+", "#", "."],
                // Nothing is filled in lazily: every item is complete when it
                // is sent, so there is no `completionItem/resolve` to answer.
                "resolveProvider": false,
            },
            "documentSymbolProvider": true,
            "workspaceSymbolProvider": true,
            // Whole-document only. `documentRangeFormattingProvider` is
            // deliberately absent: a range's own text does not say which
            // lexical area it starts in, so formatting one would mean either
            // lexing the whole file to find out (at which point the range
            // bought nothing) or guessing — and guessing here rewrites prose.
            "documentFormattingProvider": true,
            // The protocol's default, stated explicitly because it is the
            // one thing about this server most likely to be got wrong by
            // whoever touches `line_index` next: every `character` below is
            // a UTF-16 code unit.
            "positionEncoding": "utf-16",
        },
        "serverInfo": {
            "name": "rustyfi-lsp",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

/// A string field of a JSON object.
///
/// Borrowed, not owned: `text` here is the whole document, and the `params`
/// it lives in outlive every use of it, so copying the buffer to look at it
/// would be one wasted allocation of the file's size per keystroke.
fn str_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name).and_then(Value::as_str)
}

/// What a `didChange` did to the document, as far as this server can act on
/// it.
///
/// Three outcomes, and collapsing any two of them loses a document. This used
/// to be one `Option<&str>` where `None` meant "forget what you hold", which
/// is right for a ranged change and wrong for an EMPTY change list: nothing
/// changed, so the copy the server holds is still exactly the client's.
enum Change<'a> {
    /// A whole-document replacement — this is the new text.
    Full(&'a str),
    /// The change list was empty, so the document is untouched. Keep it.
    Nothing,
    /// A ranged (incremental) change under a Full-sync agreement, or a list
    /// this server cannot read. What it holds may be stale, so it must go.
    Unreadable,
}

/// Classify a `didChange`'s `contentChanges`.
///
/// A Full-sync change list is normally one entry with only a `text` member.
/// The LAST entry wins if a client batches several, since they apply in order
/// and a trailing whole-document replacement supersedes what came before it.
/// A `range` on that entry means the client is doing incremental sync
/// regardless of what was advertised, and it is reported as
/// [`Change::Unreadable`] rather than silently treated as the whole file.
fn full_replacement(params: &Value) -> Change<'_> {
    let Some(changes) = params.get("contentChanges").and_then(Value::as_array) else {
        return Change::Unreadable;
    };
    // An empty list is a no-op, not a loss. A client may send one after an
    // undo that restored the saved text, and reading it as "forget" left the
    // document unformattable — and unhoverable, and undiagnosable — until the
    // next full change arrived.
    let Some(last) = changes.last() else {
        return Change::Nothing;
    };
    if last.get("range").is_some_and(|r| !r.is_null()) {
        return Change::Unreadable;
    }
    match str_field(last, "text") {
        Some(text) => Change::Full(text),
        None => Change::Unreadable,
    }
}

#[cfg(test)]
mod minimal_edit_tests {
    use super::minimal_edit_span;

    /// Applying the edit to `old` must reconstruct `new`, and neither cut may
    /// land inside a character — in EITHER string.
    ///
    /// This is the one link between a correct `format` output and the client's
    /// buffer that nothing exercised. A wrong cut here does not produce
    /// tidy-but-odd whitespace; it silently deletes the user's text, or panics
    /// on a slice that is not a char boundary.
    fn check(old: &str, new: &str) {
        let (start, old_end, new_end) = minimal_edit_span(old, new);
        assert!(start <= old_end && old_end <= old.len(), "{old:?} -> {new:?}");
        assert!(start <= new_end && new_end <= new.len(), "{old:?} -> {new:?}");
        for (s, at) in [(old, start), (old, old_end), (new, start), (new, new_end)] {
            assert!(
                s.is_char_boundary(at),
                "cut at {at} is inside a character of {s:?} ({old:?} -> {new:?})",
            );
        }
        let applied = format!("{}{}{}", &old[..start], &new[start..new_end], &old[old_end..]);
        assert_eq!(applied, new, "applying the edit to {old:?} did not give {new:?}");
    }

    /// A deterministic xorshift, so a failure is reproducible from the seed
    /// printed in the panic rather than from a lucky rerun.
    fn rng(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// Deliberately hostile: multi-byte and astral characters (so a cut can
    /// land mid-character), both line endings, and characters that repeat so
    /// that long common prefixes and suffixes actually occur.
    ///
    /// `あ` (E3 81 82) and `も` (E3 82 82) are in here as a PAIR, and that is
    /// the point of them: they share a trailing byte without being the same
    /// character, which is what makes the byte-wise suffix scan stop in the
    /// middle of one and forces the rounding below it to matter. Without such
    /// a pair the rounding direction is unobservable — a mutation that rounds
    /// the cut the wrong way survives 200 000 cases, because every cut lands
    /// on a boundary anyway. `🎉`/`🎊` (F0 9F 8E 89 / 8A) are the astral
    /// version of the same trick.
    const ALPHABET: [&str; 14] = [
        "a", "a", "b", " ", "\n", "\r\n", "\t", "%", "あ", "も", "漢", "🎉", "🎊", "é",
    ];

    fn build(state: &mut u64, max: usize) -> String {
        let n = (rng(state) as usize) % (max + 1);
        (0..n).map(|_| ALPHABET[(rng(state) as usize) % ALPHABET.len()]).collect()
    }

    #[test]
    fn applying_the_edit_reconstructs_the_new_text() {
        // The shapes the generator is unlikely to hit on its own.
        for (old, new) in [
            ("", ""), ("", "x"), ("x", ""), ("x", "x"),
            ("あ", "い"), ("🎉", "🎊"), ("a🎉b", "a🎉c"), ("a🎉b", "ab"),
            ("\r\n", "\n"), ("\n", "\r\n"), ("aaa", "aa"), ("aa", "aaa"),
            ("漢字", "漢"), ("漢", "漢字"),
        ] {
            check(old, new);
        }
        let mut state = 0x5eed_1234_9abc_def0u64;
        for _ in 0..200_000 {
            let old = build(&mut state, 12);
            // Half the time edit `old` rather than draw independently, so long
            // shared prefixes and suffixes — the case the function exists for —
            // are actually reached.
            let new = match rng(&mut state) % 2 {
                0 => build(&mut state, 12),
                _ => {
                    let mut n = old.clone();
                    n.push_str(&build(&mut state, 3));
                    let cut = (rng(&mut state) as usize) % (n.len() + 1);
                    let cut = (0..=cut).rev().find(|c| n.is_char_boundary(*c)).unwrap_or(0);
                    n.truncate(cut);
                    n
                }
            };
            check(&old, &new);
        }
    }
}
