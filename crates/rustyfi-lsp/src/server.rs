//! The stdio language server: lifecycle, document sync, and diagnostics.
//!
//! Everything the server *knows* is in [`crate::analyze`]; this module only
//! decides when to ask and how to phrase the answer.
//!
//! # What is implemented
//!
//! `initialize`, `initialized`, `shutdown`, `exit`,
//! `textDocument/didOpen` / `didChange` / `didClose`,
//! `textDocument/publishDiagnostics`, and the three interactive requests —
//! `textDocument/hover`, `textDocument/definition` and
//! `textDocument/completion`. The `initialize` reply advertises exactly that
//! and nothing more — an over-claimed capability costs the user a hang or an
//! empty popup on every keystroke, so the reply lists only what is actually
//! wired up.
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
//! remembering is how staleness bugs are avoided. The three interactive
//! requests are *pulled* — `textDocument/hover` carries a URI and a position
//! and no text at all — so the server has to hold what the client last sent
//! it. [`State::docs`] is that and only that: it is written on
//! `didOpen`/`didChange`, dropped on `didClose`, and never derived from.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::jsonrpc::{self, code, Incoming};
use crate::model::HeaderKind;
use crate::{ByteRange, Definition, LineIndex, Position, RustyfiVersion};

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
    pub lib_root: Option<PathBuf>,
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
struct State {
    opts: Options,
    initialized: bool,
    shutdown_requested: bool,
    /// The text of each open buffer, keyed by URI — see the module comment.
    /// Nothing derived is cached beside it: a [`crate::Model`] is rebuilt per
    /// request, because a cache keyed on a buffer that changes on every
    /// keystroke is a staleness bug waiting for a race, and the parse is
    /// budgeted (`high_water`) so its cost is bounded.
    docs: HashMap<String, String>,
}

impl State {
    fn new(opts: Options) -> Self {
        State {
            opts,
            initialized: false,
            shutdown_requested: false,
            docs: HashMap::new(),
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
            // The three interactive requests. Each answers `null` — never an
            // error — when it has nothing to say: LSP treats `null` as "no
            // result", and an error response makes a client log a failure for
            // what is an ordinary outcome (a cursor on a keyword, a name from
            // a package this buffer cannot see).
            "textDocument/hover" => Ok(self.hover(&params)),
            "textDocument/definition" => Ok(self.definition(&params)),
            "textDocument/completion" => Ok(self.completion(&params)),
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
            "textDocument/didChange" => {
                let text = full_replacement(&params).map(str::to_string);
                self.remember(&params, text.as_deref());
                self.publish(&params, text.as_deref())
            }
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

    /// Publish diagnostics for `text` against the URI and version in
    /// `params.textDocument`, or nothing if either the URI or the text is
    /// missing.
    ///
    /// `text` is a parameter rather than read from `params` because that is
    /// the *only* thing `didOpen` and `didChange` disagree about; everything
    /// else — the URI, the version echo, the "drop a malformed notification
    /// silently" rule — is shared, and was previously written twice.
    /// Store what the client just sent, so a later `hover`/`definition`/
    /// `completion` — which carry a position and no text — has a buffer to
    /// answer about.
    ///
    /// A notification with no text **forgets** the buffer rather than keeping
    /// the previous one. That is a ranged `didChange` under a Full-sync
    /// agreement — the client is not honouring the advertised capability — and
    /// the text the server holds is then known to be out of date. Diagnostics
    /// may stay on screen through such a change (they are at worst
    /// mispositioned), but a hover computed from stale text *answers about
    /// characters that are not there*, and a jump computed from it lands
    /// somewhere the user did not ask for. Answering nothing is the only
    /// honest option.
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
        // configuration at all.
        if self.opts.lib_root.is_none() {
            self.opts.lib_root = options
                .and_then(|o| o.get("libRoot"))
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("RUSTYFI_LIB_ROOT").map(PathBuf::from));
        }
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

/// A [`ByteRange`] as an LSP range over `text`.
fn lsp_range(text: &str, range: ByteRange) -> Value {
    let index = LineIndex::new(text);
    let (start, end) = (index.position(range.start), index.position(range.end));
    json!({
        "start": { "line": start.line, "character": start.character },
        "end": { "line": end.line, "character": end.character },
    })
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
