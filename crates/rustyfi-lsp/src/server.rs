//! The stdio language server: lifecycle, document sync, and diagnostics.
//!
//! Everything the server *knows* is in [`crate::analyze`]; this module only
//! decides when to ask and how to phrase the answer.
//!
//! # What is implemented
//!
//! `initialize`, `initialized`, `shutdown`, `exit`,
//! `textDocument/didOpen` / `didChange` / `didClose`, and
//! `textDocument/publishDiagnostics`. The `initialize` reply advertises
//! exactly that and nothing more — an over-claimed capability costs the user
//! a hang or an empty popup on every keystroke, so the reply lists only what
//! is actually wired up.
//!
//! Document sync is **full**, not incremental. Incremental sync would mean
//! reimplementing UTF-16-range splicing over the buffer, and the whole
//! analysis re-parses the file anyway (this port's parser has no incremental
//! mode), so the only thing incremental sync could save is the bytes on the
//! wire. Full sync is the honest choice here and is advertised as such.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::jsonrpc::{self, code, Incoming};
use crate::RustyfiVersion;

/// How the server was started.
#[derive(Debug, Default, Clone, Copy)]
pub struct Options {
    /// `rustyfi lsp --lang 0.1`: force every buffer to one generation.
    ///
    /// `None` — the default — detects per file (see [`crate::analyze_auto`],
    /// which is the CLI's own entry-document rule plus a re-check for buffers
    /// that signal no version at all). An override is worth having for a
    /// project that is wholly one generation and whose library files, being
    /// signal-free, would otherwise each be parsed twice.
    pub lang: Option<RustyfiVersion>,
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
/// Notably NOT the open buffers. A full-sync server is handed the complete
/// text with every `didOpen`/`didChange`, and the analysis is a pure function
/// of that text, so a document store would be write-only state — and
/// write-only state in a server is where staleness bugs come from. The
/// document URI is likewise never parsed or resolved to a path; nothing here
/// touches the filesystem, so it is only ever an opaque key to publish
/// back against.
struct State {
    opts: Options,
    initialized: bool,
    shutdown_requested: bool,
}

impl State {
    fn new(opts: Options) -> Self {
        State {
            opts,
            initialized: false,
            shutdown_requested: false,
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
            _ => Err((code::METHOD_NOT_FOUND, format!("{method} is not supported"))),
        }
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
                let text = params.get("textDocument").and_then(|d| str_field(d, "text"));
                self.publish(&params, text)
            }
            // `full_replacement` yields `None` for a *ranged* change under a
            // Full-sync agreement: the client is not honouring the advertised
            // capability, and applying it would need the incremental splicing
            // this server does not do. Publishing nothing leaves the previous
            // diagnostics on screen — stale, but never pointing at text that
            // is not there.
            "textDocument/didChange" => {
                let text = full_replacement(&params);
                self.publish(&params, text)
            }
            "textDocument/didClose" => {
                let Some(uri) = params.get("textDocument").and_then(|d| str_field(d, "uri")) else {
                    return Vec::new();
                };
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

    /// `initializationOptions.lang`, if the client sent one and the command
    /// line did not already pin the generation.
    ///
    /// The command line wins: it is the more explicit of the two, and an
    /// editor that guesses wrong in its client config should not be able to
    /// override what the user typed when launching the server.
    fn absorb_initialization_options(&mut self, params: &Value) {
        if self.opts.lang.is_some() {
            return;
        }
        let Some(lang) = params
            .get("initializationOptions")
            .and_then(|o| o.get("lang"))
            .and_then(Value::as_str)
        else {
            return;
        };
        self.opts.lang = lang.parse::<RustyfiVersion>().ok();
    }

    /// Analyse `text` and build the whole `textDocument/publishDiagnostics`
    /// notification, envelope included.
    fn diagnostics_for(&self, uri: &str, text: &str, version: Option<&Value>) -> Value {
        let diags = match self.opts.lang {
            Some(lang) => crate::analyze(text, lang),
            None => crate::analyze_auto(text),
        };
        let items: Vec<Value> = diags
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

/// The new full text of a `didChange`, if every change in it is a whole-
/// document replacement.
///
/// A Full-sync change list is normally one entry with only a `text` member.
/// The last entry wins if a client batches several, since they apply in
/// order. A `range` on any entry means the client is doing incremental sync
/// regardless of what was advertised, and this returns `None` rather than
/// silently treating a fragment as the whole file.
fn full_replacement(params: &Value) -> Option<&str> {
    let changes = params.get("contentChanges")?.as_array()?;
    let last = changes.last()?;
    if last.get("range").is_some_and(|r| !r.is_null()) {
        return None;
    }
    str_field(last, "text")
}
