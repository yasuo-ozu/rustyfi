//! The server driven end to end over a framed byte stream.
//!
//! Every test scripts a real session through the same
//! [`rustyfi_lsp::server::run`] the `rustyfi lsp` binary calls and asserts the
//! JSON that comes back.
//!
//! The failure mode being ruled out is **a server that compiles, handshakes,
//! accepts documents and never emits a single diagnostic**, so every test that
//! opens a broken document asserts a `publishDiagnostics` arrived with a
//! non-empty list and a range in the right place.

use rustyfi_lsp::jsonrpc::{self, code};
use rustyfi_lsp::server::{self, Options};
use rustyfi_lsp::RustyfiVersion;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Frame one message through the server's own writer, so a change to the
/// framing cannot pass here by being mirrored in the test.
fn frame(msg: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    jsonrpc::write_message(&mut out, msg).expect("writing to a Vec cannot fail");
    out
}

/// Feed a scripted message list to the server and collect everything it
/// writes, plus the exit code.
fn session(msgs: &[Value]) -> (Vec<Value>, i32) {
    session_with(msgs, Options::default())
}

fn session_with(msgs: &[Value], opts: Options) -> (Vec<Value>, i32) {
    let mut input = Vec::new();
    for m in msgs {
        input.extend(frame(m));
    }
    run_on(input, opts)
}

/// The same, from raw bytes — for the malformed-input cases that cannot be
/// expressed as a list of well-formed messages.
fn run_on(input: Vec<u8>, opts: Options) -> (Vec<Value>, i32) {
    let mut reader = std::io::Cursor::new(input);
    let mut raw: Vec<u8> = Vec::new();
    let exit = server::run(&mut reader, &mut raw, opts).expect("the server must not fail on I/O");

    // Re-read the output through the same framing reader a client would use,
    // which also checks every `Content-Length` the server wrote.
    let mut cursor = std::io::Cursor::new(raw);
    let mut out = Vec::new();
    while let Some(v) = jsonrpc::read_message(&mut cursor).unwrap() {
        out.push(v);
    }
    (out, exit)
}

fn initialize(id: i64) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": { "processId": null, "rootUri": null, "capabilities": {} },
    })
}

fn did_open(uri: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "satysfi", "version": 1, "text": text,
        }},
    })
}

fn did_change(uri: &str, version: i64, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{ "text": text }],
        },
    })
}

fn shutdown(id: i64) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "shutdown"})
}

fn exit() -> Value {
    json!({"jsonrpc": "2.0", "method": "exit"})
}

/// Every `publishDiagnostics` notification in a transcript, in order.
fn publishes(out: &[Value]) -> Vec<&Value> {
    out.iter()
        .filter(|m| m["method"] == "textDocument/publishDiagnostics")
        .collect()
}

/// The `diagnostics` array of the `n`th publish.
fn diagnostics(out: &[Value], n: usize) -> &Vec<Value> {
    publishes(out)[n]["params"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("publish {n} has no diagnostics array: {out:#?}"))
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[test]
fn the_full_lifecycle_produces_a_diagnostic_and_exits_cleanly() {
    let broken = "@require: stdjabook\nlet x = 1 in\nlet y = ] in x\n";
    let (out, code) = session(&[
        initialize(1),
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        did_open("file:///doc.saty", broken),
        shutdown(2),
        exit(),
    ]);

    assert_eq!(code, 0);
    assert_eq!(out.len(), 3, "initialize reply, one publish, shutdown reply: {out:#?}");

    // 1. The initialize reply, asserted in full: the capability set is the
    //    server's contract with the editor, and an accidental addition would
    //    be a promise nothing keeps.
    assert_eq!(out[0]["jsonrpc"], "2.0");
    assert_eq!(out[0]["id"], 1);
    assert_eq!(
        out[0]["result"]["capabilities"],
        json!({
            "textDocumentSync": { "openClose": true, "change": 1 },
            "documentSymbolProvider": true,
            "workspaceSymbolProvider": true,
            "positionEncoding": "utf-16",
        }),
    );
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "rustyfi-lsp");

    // 2. The diagnostic. `]` is at character 8 of line 2 (zero-based).
    assert_eq!(
        out[1],
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///doc.saty",
                "version": 1,
                "diagnostics": [{
                    "range": {
                        "start": { "line": 2, "character": 8 },
                        "end":   { "line": 2, "character": 9 },
                    },
                    "severity": 1,
                    "source": "rustyfi",
                    "message": "too many closing",
                }],
            },
        }),
    );

    // 3. The shutdown reply: a null result, per the specification.
    assert_eq!(out[2], json!({"jsonrpc": "2.0", "id": 2, "result": null}));
}

#[test]
fn a_request_before_initialize_is_refused() {
    let (out, _) = session(&[shutdown(7)]);
    assert_eq!(out[0]["id"], 7);
    assert_eq!(out[0]["error"]["code"], code::SERVER_NOT_INITIALIZED);
}

#[test]
fn a_notification_before_initialize_is_dropped_silently() {
    let (out, _) = session(&[did_open("file:///a.saty", "let x = ] in x")]);
    assert!(out.is_empty(), "expected silence, got {out:?}");
}

#[test]
fn an_unknown_request_is_method_not_found_and_the_session_survives() {
    let (out, code_) = session(&[
        initialize(1),
        json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/hover", "params": {}}),
        shutdown(3),
        exit(),
    ]);
    assert_eq!(out[1]["error"]["code"], code::METHOD_NOT_FOUND);
    assert_eq!(out[2]["id"], 3);
    assert!(out[2]["result"].is_null());
    assert_eq!(code_, 0);
}

#[test]
fn exit_without_shutdown_is_a_failure_code() {
    let (_, code) = session(&[initialize(1), exit()]);
    assert_eq!(code, 1, "the specification requires exit code 1 here");
}

#[test]
fn a_closed_pipe_without_exit_is_a_clean_disconnect() {
    let (_, code) = session(&[initialize(1)]);
    assert_eq!(code, 0);
}

#[test]
fn nothing_is_published_after_shutdown() {
    let (out, _) = session(&[
        initialize(1),
        shutdown(2),
        did_open("file:///a.saty", "let x = ] in x"),
    ]);
    assert!(publishes(&out).is_empty(), "the session is winding down: {out:?}");
}

#[test]
fn malformed_json_gets_a_parse_error_and_the_session_survives() {
    let mut input = b"Content-Length: 5\r\n\r\n{bad}".to_vec();
    input.extend(frame(&initialize(1)));
    let (out, _) = run_on(input, Options::default());
    assert_eq!(out[0]["error"]["code"], code::PARSE_ERROR);
    assert!(out[0]["id"].is_null(), "the id is unknowable for unparsable JSON");
    assert!(out[1]["result"]["capabilities"].is_object(), "session survived");
}

// ---------------------------------------------------------------------------
// Document sync
// ---------------------------------------------------------------------------

#[test]
fn a_valid_document_gets_an_empty_diagnostic_list_not_silence() {
    // A server that publishes nothing for a clean file leaves whatever the
    // editor last showed on screen.
    let (out, _) = session(&[
        initialize(1),
        did_open(
            "file:///ok.saty",
            "@require: stdjabook\ndocument (| title = `t` |) '<\n  +p { fine }\n>\n",
        ),
    ]);
    assert_eq!(publishes(&out).len(), 1);
    assert!(diagnostics(&out, 0).is_empty());
}

#[test]
fn editing_a_document_republishes_from_the_new_text() {
    let uri = "file:///edit.saty";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, "@require: stdjabook\nlet x = ] in x\n"),
        did_change(uri, 2, "@require: stdjabook\nlet x = 1 in x\n"),
        did_change(uri, 3, "@require: stdjabook\nlet x = 1 in ]\n"),
    ]);
    let pubs = publishes(&out);
    assert_eq!(pubs.len(), 3);

    assert_eq!(pubs[0]["params"]["version"], 1);
    assert_eq!(diagnostics(&out, 0)[0]["range"]["start"]["character"], 8);

    // The fix is reflected: an empty list retracts the previous squiggle.
    assert_eq!(pubs[1]["params"]["version"], 2);
    assert!(diagnostics(&out, 1).is_empty());

    // And a new error at a new place is reported at that place.
    assert_eq!(pubs[2]["params"]["version"], 3);
    assert_eq!(diagnostics(&out, 2)[0]["range"]["start"]["character"], 13);
}

#[test]
fn did_close_retracts_the_diagnostics() {
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///a.saty", "let x = ] in x"),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "file:///a.saty" }},
        }),
    ]);
    assert!(!diagnostics(&out, 0).is_empty());
    let last = publishes(&out)[1];
    assert_eq!(last["params"]["uri"], "file:///a.saty");
    assert!(diagnostics(&out, 1).is_empty());
}

#[test]
fn a_ranged_change_is_ignored_rather_than_misapplied() {
    // Full sync was advertised; a client sending a fragment must not have it
    // mistaken for the whole buffer.
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///a.saty", "let x = 1 in x"),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": "file:///a.saty", "version": 2 },
                "contentChanges": [{
                    "range": {"start": {"line": 0, "character": 0},
                              "end": {"line": 0, "character": 1}},
                    "text": "]",
                }],
            },
        }),
    ]);
    assert_eq!(publishes(&out).len(), 1, "only didOpen published: {out:#?}");
}

// ---------------------------------------------------------------------------
// Which generation a buffer is read as
// ---------------------------------------------------------------------------

#[test]
fn a_0_1_document_is_analysed_with_the_0_1_grammar() {
    // The likeliest way this server ends up subtly useless: reading a 0.1
    // buffer with the 0.0.6 parser and painting nonsense over a file that
    // compiles. Both halves are asserted.
    let clean = "@require: basic\n\
                 module M :> sig\n\
                   val double : int -> int\n\
                 end = struct\n\
                   val double n = n * 2\n\
                 end\n";
    let (out, _) = session(&[initialize(1), did_open("file:///lib.satyh", clean)]);
    assert!(diagnostics(&out, 0).is_empty(), "{out:#?}");

    let broken = "@require: basic\nmodule M = struct\n  val a = = 1\nend\n";
    let (out, _) = session(&[initialize(1), did_open("file:///lib.satyh", broken)]);
    let diags = diagnostics(&out, 0);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0]["range"]["start"], json!({ "line": 2, "character": 10 }));
}

#[test]
fn a_lang_override_is_honoured_over_the_buffers_own_signal() {
    // `let-rec` sniffs as 0.0.6. Forced to 0.1 it must not parse, which is how
    // we know the override reached the analysis rather than being dropped.
    let src = "let-rec f x = x\n";
    let opts = Options {
        lang: Some(RustyfiVersion::V0_1),
        ..Default::default()
    };
    let (out, _) = session_with(&[initialize(1), did_open("file:///f.saty", src)], opts);
    assert!(
        !diagnostics(&out, 0).is_empty(),
        "the 0.1 grammar has no `let-rec`, so forcing 0.1 must report it"
    );

    // The same buffer with no override is clean.
    let (out, _) = session(&[initialize(1), did_open("file:///f.saty", src)]);
    assert!(diagnostics(&out, 0).is_empty());
}

#[test]
fn a_bad_initialization_option_neither_crashes_nor_pins() {
    let (out, _) = session(&[
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "initializationOptions": { "lang": "nonsense" }},
        }),
        did_open("file:///a.saty", "@require: p\nlet x = 1 in x\n"),
    ]);
    assert!(diagnostics(&out, 0).is_empty());
}

// ---------------------------------------------------------------------------
// Positions on the wire
// ---------------------------------------------------------------------------

#[test]
fn utf16_columns_survive_the_wire() {
    // The end-to-end version of the column check: a diagnostic after
    // Japanese text must arrive at the UTF-16 column, not the byte offset.
    let src = "let x = `こんにちは` in let y = ] in y";
    assert_eq!(src.find(']'), Some(37), "byte offset, for contrast");
    let (out, _) = session(&[initialize(1), did_open("file:///jp.saty", src)]);
    assert_eq!(
        diagnostics(&out, 0)[0]["range"],
        json!({
            "start": { "line": 0, "character": 27 },
            "end":   { "line": 0, "character": 28 },
        }),
    );
}

#[test]
fn crlf_line_numbers_agree_with_the_lexers_own_rule() {
    // `LineIndex` counts lines itself rather than trusting `Loc::line`, so the
    // two definitions of "what terminates a line" have to agree. They differ
    // only on CRLF, where a naive implementation counts `\r\n` as two lines,
    // and every other fixture in both crates is `\n`-only.
    let src = "@require: stdjabook\r\nlet x = 1 in\r\nlet y = ] in x\r\n";
    let (out, _) = session(&[initialize(1), did_open("file:///crlf.saty", src)]);
    assert_eq!(
        diagnostics(&out, 0)[0]["range"]["start"],
        json!({ "line": 2, "character": 8 }),
        "CRLF must not shift the line number: {out:#?}"
    );
}

// ---------------------------------------------------------------------------
// textDocument/documentSymbol
// ---------------------------------------------------------------------------

fn document_symbol(id: i64, uri: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/documentSymbol",
        "params": { "textDocument": { "uri": uri } },
    })
}

/// The reply to the request with this id.
fn reply(out: &[Value], id: i64) -> &Value {
    out.iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("no reply with id {id}: {out:#?}"))
}

/// The whole exchange, asserted as JSON: the one place a field-name slip
/// (`selectionRange` written `selection_range`) shows up as a broken outline
/// rather than as a type error.
#[test]
fn document_symbol_returns_the_outline_of_an_open_buffer() {
    let src = "@require: list\n\nmodule M = struct\n  let f x = x\nend\n";
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///lib.satyh", src),
        document_symbol(2, "file:///lib.satyh"),
    ]);

    assert_eq!(
        reply(&out, 2)["result"],
        json!([
            {
                "name": "list",
                "detail": "@require:",
                // 4 = Package.
                "kind": 4,
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end":   { "line": 0, "character": 14 },
                },
                "selectionRange": {
                    "start": { "line": 0, "character": 0 },
                    "end":   { "line": 0, "character": 14 },
                },
            },
            {
                "name": "M",
                "detail": "module",
                // 2 = Module.
                "kind": 2,
                "range": {
                    "start": { "line": 2, "character": 0 },
                    "end":   { "line": 4, "character": 3 },
                },
                "selectionRange": {
                    "start": { "line": 2, "character": 7 },
                    "end":   { "line": 2, "character": 8 },
                },
                "children": [{
                    "name": "f",
                    "detail": "let",
                    // 12 = Function.
                    "kind": 12,
                    "range": {
                        "start": { "line": 3, "character": 2 },
                        "end":   { "line": 3, "character": 13 },
                    },
                    "selectionRange": {
                        "start": { "line": 3, "character": 6 },
                        "end":   { "line": 3, "character": 7 },
                    },
                }],
            },
        ]),
    );
}

/// A `didChange` replaces the stored buffer, ruling out a symbol pane that is
/// correct once and then frozen.
#[test]
fn document_symbol_follows_did_change() {
    let uri = "file:///doc.satyh";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, "let before = 1\n"),
        did_change(uri, 2, "let after = 1\nlet also = 2\n"),
        document_symbol(3, uri),
    ]);

    let names: Vec<&str> = reply(&out, 3)["result"]
        .as_array()
        .expect("an array of symbols")
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["after", "also"]);
}

/// A URI the server was never given — never opened, or closed again — gets an
/// empty outline rather than an error.
#[test]
fn document_symbol_on_an_unknown_uri_is_an_empty_outline() {
    let uri = "file:///gone.satyh";
    let (out, _) = session(&[
        initialize(1),
        document_symbol(2, "file:///never-opened.satyh"),
        did_open(uri, "let x = 1\n"),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } },
        }),
        document_symbol(3, uri),
    ]);

    assert_eq!(reply(&out, 2)["result"], json!([]));
    assert_eq!(reply(&out, 3)["result"], json!([]), "didClose must forget");
}

/// The UTF-16 rule on the outline path: `title`'s value is 42 bytes of
/// Japanese, so `sub`'s columns are only right if the server counts code
/// units.
#[test]
fn document_symbol_columns_are_utf16_over_the_wire() {
    let src = "let title = `日本語のタイトル` let sub = 2\n";
    assert_eq!(src.find("sub ="), Some(43), "byte offset, for contrast");
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///jp.satyh", src),
        document_symbol(2, "file:///jp.satyh"),
    ]);

    assert_eq!(
        reply(&out, 2)["result"][1]["selectionRange"],
        json!({
            "start": { "line": 0, "character": 27 },
            "end":   { "line": 0, "character": 30 },
        }),
    );
}

/// `--lang` pins the generation for the outline as it does for diagnostics: a
/// 0.1 library asked for as 0.0.6 declares nothing, and the server must not
/// quietly fall back to the reading that works better.
#[test]
fn document_symbol_obeys_an_explicit_lang() {
    let src = "module Lib = struct\n  val f x = x\nend\n";
    let uri = "file:///lib.satyh";
    let msgs = [initialize(1), did_open(uri, src), document_symbol(2, uri)];

    let (auto, _) = session(&msgs);
    assert_eq!(reply(&auto, 2)["result"][0]["name"], "Lib");
    assert_eq!(
        reply(&auto, 2)["result"][0]["children"][0]["name"],
        "f",
        "the ambiguity re-check must read this as 0.1"
    );

    let (pinned, _) = session_with(
        &msgs,
        Options {
            lang: Some(RustyfiVersion::V0_0),
            ..Default::default()
        },
    );
    assert_eq!(reply(&pinned, 2)["result"], json!([]));
}
