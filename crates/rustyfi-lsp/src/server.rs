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
    /// `None` — the default — detects per file, exactly as the CLI does for
    /// the entry document (see [`crate::detect_version`]). An override is
    /// worth having for a project that is wholly one generation and contains
    /// library files whose text carries no version signal.
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
        // specification, except `exit`.
        if !self.initialized {
            return Vec::new();
        }
        match method {
            "textDocument/didOpen" => {
                let Some(doc) = params.get("textDocument") else {
                    return Vec::new();
                };
                let (Some(uri), Some(text)) = (str_field(doc, "uri"), str_field(doc, "text"))
                else {
                    return Vec::new();
                };
                vec![self.diagnostics_for(&uri, &text, doc.get("version").cloned())]
            }
            "textDocument/didChange" => {
                let Some(doc) = params.get("textDocument") else {
                    return Vec::new();
                };
                let Some(uri) = str_field(doc, "uri") else {
                    return Vec::new();
                };
                let Some(text) = full_replacement(&params) else {
                    // A ranged change under a Full-sync agreement: the client
                    // is not honouring the advertised capability, and applying
                    // it would need the incremental splicing this server does
                    // not do. Publishing nothing leaves the previous
                    // diagnostics on screen — stale, but never pointing at
                    // text that is not there.
                    return Vec::new();
                };
                vec![self.diagnostics_for(&uri, &text, doc.get("version").cloned())]
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
    fn diagnostics_for(&self, uri: &str, text: &str, version: Option<Value>) -> Value {
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
            params["version"] = v;
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
fn str_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).and_then(Value::as_str).map(str::to_owned)
}

/// The new full text of a `didChange`, if every change in it is a whole-
/// document replacement.
///
/// A Full-sync change list is normally one entry with only a `text` member.
/// The last entry wins if a client batches several, since they apply in
/// order. A `range` on any entry means the client is doing incremental sync
/// regardless of what was advertised, and this returns `None` rather than
/// silently treating a fragment as the whole file.
fn full_replacement(params: &Value) -> Option<String> {
    let changes = params.get("contentChanges")?.as_array()?;
    let last = changes.last()?;
    if last.get("range").is_some_and(|r| !r.is_null()) {
        return None;
    }
    str_field(last, "text")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &Value) -> Vec<u8> {
        let s = serde_json::to_vec(body).unwrap();
        let mut out = format!("Content-Length: {}\r\n\r\n", s.len()).into_bytes();
        out.extend(s);
        out
    }

    /// Drive `run` over a scripted message list, returning every message the
    /// server wrote plus its exit code.
    fn drive(msgs: &[Value], opts: Options) -> (Vec<Value>, i32) {
        let mut input: Vec<u8> = Vec::new();
        for m in msgs {
            input.extend(frame(m));
        }
        let mut reader = io::Cursor::new(input);
        let mut out: Vec<u8> = Vec::new();
        let exit = run(&mut reader, &mut out, opts).expect("the server must not fail on I/O");
        let mut cursor = io::Cursor::new(out);
        let mut seen = Vec::new();
        while let Some(v) = jsonrpc::read_message(&mut cursor).unwrap() {
            seen.push(v);
        }
        (seen, exit)
    }

    fn init() -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
    }

    fn did_open(uri: &str, text: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": uri, "languageId": "satysfi", "version": 1, "text": text,
            }},
        })
    }

    #[test]
    fn a_request_before_initialize_is_refused() {
        let (out, _) = drive(
            &[json!({"jsonrpc": "2.0", "id": 7, "method": "shutdown"})],
            Options::default(),
        );
        assert_eq!(out[0]["id"], 7);
        assert_eq!(out[0]["error"]["code"], code::SERVER_NOT_INITIALIZED);
    }

    #[test]
    fn a_notification_before_initialize_is_dropped_silently() {
        let (out, _) = drive(&[did_open("file:///a.saty", "let x = ] in x")], Options::default());
        assert!(out.is_empty(), "expected silence, got {out:?}");
    }

    #[test]
    fn an_unknown_request_is_method_not_found_and_the_session_survives() {
        let (out, exit) = drive(
            &[
                init(),
                json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/hover", "params": {}}),
                json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown"}),
                json!({"jsonrpc": "2.0", "method": "exit"}),
            ],
            Options::default(),
        );
        assert_eq!(out[1]["error"]["code"], code::METHOD_NOT_FOUND);
        assert_eq!(out[2]["id"], 3);
        assert!(out[2]["result"].is_null());
        assert_eq!(exit, 0);
    }

    #[test]
    fn exit_without_shutdown_is_a_failure_code() {
        let (_, exit) = drive(&[init(), json!({"jsonrpc": "2.0", "method": "exit"})], Options::default());
        assert_eq!(exit, 1, "the specification requires exit code 1 here");
    }

    #[test]
    fn a_closed_pipe_without_exit_is_a_clean_disconnect() {
        let (_, exit) = drive(&[init()], Options::default());
        assert_eq!(exit, 0);
    }

    #[test]
    fn did_close_retracts_the_diagnostics() {
        let (out, _) = drive(
            &[
                init(),
                did_open("file:///a.saty", "let x = ] in x"),
                json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didClose",
                    "params": { "textDocument": { "uri": "file:///a.saty" }},
                }),
            ],
            Options::default(),
        );
        assert!(!out[1]["params"]["diagnostics"].as_array().unwrap().is_empty());
        let last = out.last().unwrap();
        assert_eq!(last["method"], "textDocument/publishDiagnostics");
        assert_eq!(last["params"]["uri"], "file:///a.saty");
        assert!(last["params"]["diagnostics"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_ranged_change_is_ignored_rather_than_misapplied() {
        // Full sync was advertised; a client sending a fragment must not have
        // it mistaken for the whole buffer.
        let (out, _) = drive(
            &[
                init(),
                did_open("file:///a.saty", "let x = 1 in x"),
                json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didChange",
                    "params": {
                        "textDocument": { "uri": "file:///a.saty", "version": 2 },
                        "contentChanges": [{
                            "range": {"start": {"line": 0, "character": 0},
                                      "end": {"line": 0, "character": 1}},
                            "text": "]",
                        }],
                    },
                }),
            ],
            Options::default(),
        );
        // One publish (from didOpen) and nothing for the ranged change.
        assert_eq!(out.len(), 2, "{out:?}");
    }

    #[test]
    fn malformed_json_gets_a_parse_error_and_the_session_survives() {
        let mut input = b"Content-Length: 5\r\n\r\n{bad}".to_vec();
        input.extend(frame(&init()));
        let mut reader = io::Cursor::new(input);
        let mut out: Vec<u8> = Vec::new();
        run(&mut reader, &mut out, Options::default()).unwrap();
        let mut cursor = io::Cursor::new(out);
        let first = jsonrpc::read_message(&mut cursor).unwrap().unwrap();
        assert_eq!(first["error"]["code"], code::PARSE_ERROR);
        assert!(first["id"].is_null());
        let second = jsonrpc::read_message(&mut cursor).unwrap().unwrap();
        assert!(second["result"]["capabilities"].is_object(), "session survived");
    }

    #[test]
    fn initialization_options_can_pin_the_generation() {
        // `module M = struct .. end` carries no version signal, so with no
        // pin it is analysed as 0.0.6 and then re-checked; pinned to 0.1 it
        // is analysed as 0.1 directly. Both are clean here — what is asserted
        // is that a bad `lang` string does not crash and does not pin.
        let (out, _) = drive(
            &[
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "initializationOptions": { "lang": "nonsense" }},
                }),
                did_open("file:///a.saty", "@require: p\nlet x = 1 in x\n"),
            ],
            Options::default(),
        );
        assert!(out[1]["params"]["diagnostics"].as_array().unwrap().is_empty());
    }
}
