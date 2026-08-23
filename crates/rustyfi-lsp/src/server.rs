//! The stdio language server: lifecycle, document sync, and diagnostics.
//!
//! Everything the server *knows* is in [`crate::analyze`] and
//! [`crate::project::check`]; this module only decides which of the two to
//! ask, and how to phrase the answer.
//!
//! # What is implemented
//!
//! `initialize`, `initialized`, `shutdown`, `exit`, `textDocument/didOpen` /
//! `didChange` / `didClose`, `textDocument/publishDiagnostics`,
//! `textDocument/documentSymbol` and `workspace/symbol`. The `initialize`
//! reply advertises exactly that and nothing more — an over-claimed capability
//! costs the user a hang or an empty popup on every keystroke.
//!
//! Everything but `workspace/symbol` answers from the text the client sent.
//! `workspace/symbol` reads the project's other files and is the module's only
//! filesystem access — see the crate-private `workspace` module.
//!
//! Document sync is **full**, not incremental. Incremental sync would mean
//! UTF-16-range splicing over the buffer, and the analysis re-parses the file
//! anyway (this port's parser has no incremental mode), so all it could save
//! is bytes on the wire.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::jsonrpc::{self, code, Incoming};
use crate::workspace::Workspace;
use crate::{RustyfiVersion, Symbol};

/// How the server was started.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// `rustyfi lsp --lang 0.1`: force every buffer to one generation.
    ///
    /// `None` — the default — detects per file (see [`crate::analyze_auto`]).
    /// An override saves a second parse per signal-free library file in a
    /// project that is wholly one generation.
    pub lang: Option<RustyfiVersion>,

    /// Whole-program analysis: resolve each buffer's `@require:`/`@import:`
    /// graph and typecheck it, not just parse it ([`crate::project::check`]).
    ///
    /// `None` — the default — is the parse tier alone, which is what a
    /// consumer with no filesystem to offer wants. `rustyfi lsp` sets
    /// `Some(..)` unless `--no-typecheck` is given, and fills in the root
    /// discovery this crate deliberately cannot do (see
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
/// Streams rather than `stdin()`/`stdout()` so a test can drive the whole
/// protocol in-process over the same code path the binary runs.
///
/// The exit code follows the specification's `exit` rules: `0` if `shutdown`
/// came first, `1` otherwise. A client that just closes the pipe is a clean
/// disconnect (`0`) — that is how an editor goes away when it is killed.
pub fn run(input: &mut impl BufRead, output: &mut impl Write, opts: Options) -> io::Result<i32> {
    let mut state = State::new(opts);
    loop {
        let msg = match jsonrpc::read_message(input) {
            Ok(Some(msg)) => msg,
            Ok(None) => return Ok(0),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                // Malformed JSON: a null-id parse error is the only response
                // JSON-RPC allows when the id is unknowable. The session lives.
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

/// Everything the server remembers between messages, including the open
/// buffers.
///
/// Diagnostics are *pushed* from the text a notification carries and would
/// need no store, but a `textDocument/documentSymbol` **request** carries only
/// a URI. Reading that URI back off disk would answer about the saved file
/// rather than the buffer being edited, and would put filesystem access in a
/// server that otherwise has none.
///
/// The store is keyed by the URI as an opaque string, never resolved to a
/// path except by the whole-program tier ([`crate::project::path_from_uri`]).
/// Entries live from `didOpen` to `didClose`, exactly the window the
/// specification says a server may be asked about a document in.
struct State {
    opts: Options,
    initialized: bool,
    shutdown_requested: bool,
    /// Open buffers, by URI. Replaced wholesale on every `didChange` (sync is
    /// Full), so it cannot go stale against what the client has.
    docs: HashMap<String, String>,
    /// The project's folders and their cached outlines, for
    /// `workspace/symbol`. The only part of this server that reads the
    /// filesystem — see the `workspace` module.
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
            "textDocument/documentSymbol" => Ok(self.document_symbols(&params)),
            "workspace/symbol" => {
                let query = params.get("query").and_then(Value::as_str).unwrap_or("");
                let lang = self.opts.lang;
                Ok(Value::Array(self.workspace.query(query, &self.docs, lang)))
            }
            _ => Err((code::METHOD_NOT_FOUND, format!("{method} is not supported"))),
        }
    }

    /// Handle a notification, returning any messages to send as a result.
    ///
    /// `exit` is handled by the loop, not here, because it ends the process
    /// rather than producing a message.
    fn notification(&mut self, method: &str, params: Value) -> Vec<Value> {
        // The specification says notifications before `initialize` should be
        // dropped, `exit` excepted. The same goes once `shutdown` has been
        // answered: the client has stopped listening.
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
            // capability. Publishing nothing leaves the previous diagnostics
            // on screen — stale, but never pointing at text that is not there.
            "textDocument/didChange" => {
                let text = full_replacement(&params).map(str::to_string);
                self.remember(&params, text.as_deref());
                self.publish(&params, text.as_deref())
            }
            "textDocument/didClose" => {
                let Some(uri) = params.get("textDocument").and_then(|d| str_field(d, "uri")) else {
                    return Vec::new();
                };
                self.docs.remove(uri);
                // An empty list is how a server retracts diagnostics; without
                // it the editor keeps showing them for a file that is gone.
                vec![jsonrpc::notification(
                    "textDocument/publishDiagnostics",
                    json!({ "uri": uri, "diagnostics": [] }),
                )]
            }
            // `initialized`, `$/cancelRequest`, `$/setTrace` and anything
            // else: silently ignored, as the specification asks.
            _ => Vec::new(),
        }
    }

    /// Publish diagnostics for `text` against the URI and version in
    /// `params.textDocument`, or nothing if either is missing.
    ///
    /// `text` is a parameter rather than read from `params` because it is the
    /// *only* thing `didOpen` and `didChange` disagree about.
    fn publish(&self, params: &Value, text: Option<&str>) -> Vec<Value> {
        let Some(doc) = params.get("textDocument") else {
            return Vec::new();
        };
        let (Some(uri), Some(text)) = (str_field(doc, "uri"), text) else {
            return Vec::new();
        };
        vec![self.diagnostics_for(uri, text, doc.get("version"))]
    }

    /// Store (or replace) the text of the document `params` names.
    ///
    /// A `didChange` this server cannot apply — a *ranged* change under a
    /// Full-sync agreement — arrives here as `None`, and the previous text is
    /// then **dropped** rather than kept: a `documentSymbol` answering about
    /// text the client has already edited past is harder to notice than a
    /// missing outline.
    fn remember(&mut self, params: &Value, text: Option<&str>) {
        let Some(uri) = params.get("textDocument").and_then(|d| str_field(d, "uri")) else {
            return;
        };
        match text {
            Some(text) => {
                self.docs.insert(uri.to_string(), text.to_string());
            }
            None => {
                self.docs.remove(uri);
            }
        }
    }

    /// `textDocument/documentSymbol`: the outline of one open buffer.
    ///
    /// A URI this server was never given is answered with an empty list rather
    /// than the `null` the specification also allows, so an editor does not
    /// show "request failed" in its outline pane for an unopened file.
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

    /// Apply the settings a client may send in `initializationOptions`.
    ///
    /// The command line wins on every one of them, so an editor's client
    /// config cannot override what the user typed when launching the server.
    /// For the flag-shaped options the command line only ever *disables*,
    /// which is why each is applied here in one direction only.
    fn absorb_initialization_options(&mut self, params: &Value) {
        let Some(o) = params.get("initializationOptions") else {
            return;
        };
        if self.opts.lang.is_none() {
            if let Some(lang) = o.get("lang").and_then(Value::as_str) {
                self.opts.lang = lang.parse::<RustyfiVersion>().ok();
            }
        }
        #[cfg(feature = "typecheck")]
        {
            // `typecheck: false` turns the whole tier off; nothing here can
            // turn it ON, because the roots and the discovery hook come from
            // the process that started the server.
            if o.get("typecheck") == Some(&Value::Bool(false)) {
                self.opts.project = None;
            }
            if let Some(project) = &mut self.opts.project {
                if o.get("checkLibraries") == Some(&Value::Bool(true)) {
                    project.check_libraries = true;
                }
                // `libRoot` accepts a string or an array: the loader's search
                // path is a list. Named roots replace discovery entirely,
                // exactly as `--lib-root` does for the compiler.
                let roots: Vec<std::path::PathBuf> = match o.get("libRoot") {
                    Some(Value::String(s)) => vec![s.into()],
                    Some(Value::Array(items)) => {
                        items.iter().filter_map(Value::as_str).map(Into::into).collect()
                    }
                    _ => Vec::new(),
                };
                if !roots.is_empty() && project.lib_roots.is_empty() {
                    project.lib_roots = roots;
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
    /// A non-`file:` URI (`untitled:`, an editor's scratch scheme) has no
    /// path, so there is no directory to resolve `@import:` against and no
    /// library root to discover; that buffer gets the parse tier.
    #[cfg(feature = "typecheck")]
    fn diagnose(&self, uri: &str, text: &str) -> Vec<crate::Diag> {
        let Some(project) = &self.opts.project else {
            return self.parse_only(text);
        };
        let Some(path) = crate::project::path_from_uri(uri) else {
            return self.parse_only(text);
        };
        let opts = crate::project::CheckOptions {
            // `--lang` wins over the copy the caller put in `project`, so the
            // two halves cannot disagree about a buffer's generation.
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
/// `detail` and `children` are optional in the protocol and omitted when
/// empty, which is a real fraction of the payload for a big signature.
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

/// The `initialize` result.
fn server_capabilities() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": {
                "openClose": true,
                // 1 = Full. See the module comment for why not Incremental.
                "change": 1,
            },
            "documentSymbolProvider": true,
            "workspaceSymbolProvider": true,
            // The protocol's default, stated explicitly: every `character`
            // this server emits is a UTF-16 code unit.
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
/// Borrowed, not owned: one of these fields is the whole document text, and
/// copying it to look at it would cost a file-sized allocation per keystroke.
fn str_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name).and_then(Value::as_str)
}

/// The new full text of a `didChange`, if it is a whole-document replacement.
///
/// The last entry wins if a client batches several, since they apply in order.
/// A `range` on it means the client is doing incremental sync regardless of
/// what was advertised, and this answers `None` rather than silently treating
/// a fragment as the whole file.
fn full_replacement(params: &Value) -> Option<&str> {
    let changes = params.get("contentChanges")?.as_array()?;
    let last = changes.last()?;
    if last.get("range").is_some_and(|r| !r.is_null()) {
        return None;
    }
    str_field(last, "text")
}
