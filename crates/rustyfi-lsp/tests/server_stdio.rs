//! The server driven end to end over a framed byte stream.
//!
//! This scripts a real session — `initialize`, `initialized`, `didOpen`,
//! `didChange`, `shutdown`, `exit` — through the same
//! [`rustyfi_lsp::server::run`] the `rustyfi lsp` binary calls, and asserts
//! the JSON that comes back byte for byte where the exact shape matters.
//!
//! The failure mode being ruled out is the one that costs a whole afternoon
//! in an editor: **a server that compiles, handshakes, accepts documents and
//! never emits a single diagnostic.** Every test here that opens a broken
//! document asserts a `publishDiagnostics` arrived with a non-empty list and
//! a range in the right place, so that path cannot silently go dead.

use serde_json::{json, Value};

/// Frame one message the way the base protocol requires.
fn frame(body: &Value) -> Vec<u8> {
    let s = serde_json::to_vec(body).unwrap();
    let mut out = format!("Content-Length: {}\r\n\r\n", s.len()).into_bytes();
    out.extend(s);
    out
}

/// Feed `msgs` to the server and collect everything it writes.
fn session(msgs: &[Value]) -> (Vec<Value>, i32) {
    session_with(msgs, rustyfi_lsp::server::Options::default())
}

fn session_with(msgs: &[Value], opts: rustyfi_lsp::server::Options) -> (Vec<Value>, i32) {
    let mut input = Vec::new();
    for m in msgs {
        input.extend(frame(m));
    }
    let mut reader = std::io::Cursor::new(input);
    let mut raw: Vec<u8> = Vec::new();
    let exit = rustyfi_lsp::server::run(&mut reader, &mut raw, opts).expect("no I/O failure");

    // Re-read the output through the same framing reader the client would
    // use, which also checks that every `Content-Length` we wrote is right.
    let mut cursor = std::io::Cursor::new(raw);
    let mut out = Vec::new();
    while let Some(v) = rustyfi_lsp::jsonrpc::read_message(&mut cursor).unwrap() {
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

/// Every `publishDiagnostics` notification in a transcript, in order.
fn publishes(out: &[Value]) -> Vec<&Value> {
    out.iter()
        .filter(|m| m["method"] == "textDocument/publishDiagnostics")
        .collect()
}

// ---------------------------------------------------------------------------

#[test]
fn the_full_lifecycle_produces_a_diagnostic_and_exits_cleanly() {
    let broken = "@require: stdjabook\nlet x = 1 in\nlet y = ] in x\n";
    let (out, exit) = session(&[
        initialize(1),
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        did_open("file:///doc.saty", broken),
        json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ]);

    assert_eq!(exit, 0);
    assert_eq!(out.len(), 3, "initialize reply, one publish, shutdown reply: {out:#?}");

    // 1. The initialize reply, asserted in full: the capability set is the
    //    server's contract with the editor, and an accidental addition here
    //    would be a promise nothing keeps.
    assert_eq!(out[0]["jsonrpc"], "2.0");
    assert_eq!(out[0]["id"], 1);
    assert_eq!(
        out[0]["result"]["capabilities"],
        json!({
            "textDocumentSync": { "openClose": true, "change": 1 },
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
    let pubs = publishes(&out);
    assert_eq!(pubs.len(), 1);
    assert_eq!(pubs[0]["params"]["diagnostics"], json!([]));
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
    assert_eq!(pubs[0]["params"]["diagnostics"][0]["range"]["start"]["character"], 8);

    // The fix is reflected: an empty list retracts the previous squiggle.
    assert_eq!(pubs[1]["params"]["version"], 2);
    assert_eq!(pubs[1]["params"]["diagnostics"], json!([]));

    // And a new error at a new place is reported at that place.
    assert_eq!(pubs[2]["params"]["version"], 3);
    assert_eq!(pubs[2]["params"]["diagnostics"][0]["range"]["start"]["character"], 13);
}

#[test]
fn a_0_1_document_is_analysed_with_the_0_1_grammar() {
    // The single most likely way this server ends up subtly useless: reading
    // a 0.1 buffer with the 0.0.6 parser and painting nonsense over a file
    // that compiles. Both halves are asserted — the clean file stays clean,
    // and a real 0.1 error is still caught.
    let clean = "@require: basic\n\
                 module M :> sig\n\
                   val double : int -> int\n\
                 end = struct\n\
                   val double n = n * 2\n\
                 end\n";
    let (out, _) = session(&[initialize(1), did_open("file:///lib.satyh", clean)]);
    assert_eq!(publishes(&out)[0]["params"]["diagnostics"], json!([]));

    let broken = "@require: basic\nmodule M = struct\n  val a = = 1\nend\n";
    let (out, _) = session(&[initialize(1), did_open("file:///lib.satyh", broken)]);
    let diags = &publishes(&out)[0]["params"]["diagnostics"];
    assert_eq!(diags.as_array().map(Vec::len), Some(1), "{diags}");
    assert_eq!(diags[0]["range"]["start"], json!({ "line": 2, "character": 10 }));
}

#[test]
fn a_lang_override_is_honoured_over_the_buffers_own_signal() {
    // `let-rec` sniffs as 0.0.6. Forced to 0.1 it must not parse, which is
    // how we know the override reached the analysis rather than being
    // quietly dropped.
    let src = "let-rec f x = x\n";
    let opts = rustyfi_lsp::server::Options {
        lang: Some(rustyfi_lsp::RustyfiVersion::V0_1),
    };
    let (out, _) = session_with(&[initialize(1), did_open("file:///f.saty", src)], opts);
    assert!(
        !publishes(&out)[0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty(),
        "the 0.1 grammar has no `let-rec`, so forcing 0.1 must report it"
    );

    // The same buffer with no override is clean.
    let (out, _) = session(&[initialize(1), did_open("file:///f.saty", src)]);
    assert_eq!(publishes(&out)[0]["params"]["diagnostics"], json!([]));
}

#[test]
fn utf16_columns_survive_the_wire() {
    // The end-to-end version of the column check: a diagnostic after
    // Japanese text must arrive at the UTF-16 column, not the byte offset.
    let src = "let x = `こんにちは` in let y = ] in y";
    assert_eq!(src.find(']'), Some(37), "byte offset, for contrast");
    let (out, _) = session(&[initialize(1), did_open("file:///jp.saty", src)]);
    assert_eq!(
        publishes(&out)[0]["params"]["diagnostics"][0]["range"],
        json!({
            "start": { "line": 0, "character": 27 },
            "end":   { "line": 0, "character": 28 },
        }),
    );
}
