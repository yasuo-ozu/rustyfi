//! The server driven end to end over a framed byte stream.
//!
//! Everything here scripts a real session through the same
//! [`rustyfi_lsp::server::run`] the `rustyfi lsp` binary calls, and asserts
//! the JSON that comes back — byte for byte where the exact shape matters.
//! `run` takes streams rather than `stdin()`/`stdout()` precisely so this can
//! be the real code path rather than a re-implementation of it, and the whole
//! server surface is public, so none of it needs to be a `#[cfg(test)]` module
//! inside `server.rs` with a second copy of this harness.
//!
//! The failure mode being ruled out is the one that costs a whole afternoon
//! in an editor: **a server that compiles, handshakes, accepts documents and
//! never emits a single diagnostic.** Every test here that opens a broken
//! document asserts a `publishDiagnostics` arrived with a non-empty list and
//! a range in the right place, so that path cannot silently go dead.

use rustyfi_lsp::jsonrpc::{self, code};
use rustyfi_lsp::server::{self, Options};
use rustyfi_lsp::{LineIndex, Position, RustyfiVersion};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Frame one message the way the base protocol requires — through the
/// server's own writer, so a change to the framing cannot pass here by being
/// mirrored in the test.
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
    //    server's contract with the editor, and an accidental addition here
    //    would be a promise nothing keeps.
    assert_eq!(out[0]["jsonrpc"], "2.0");
    assert_eq!(out[0]["id"], 1);
    assert_eq!(
        out[0]["result"]["capabilities"],
        json!({
            "textDocumentSync": { "openClose": true, "change": 1 },
            "hoverProvider": true,
            "definitionProvider": true,
            "completionProvider": {
                "triggerCharacters": ["\\", "+", "#", "."],
                "resolveProvider": false,
            },
            "documentSymbolProvider": true,
            "workspaceSymbolProvider": true,
            "documentFormattingProvider": true,
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
                    // The lexer's own words, behind the framing every
                    // diagnostic in this port now shares. It read
                    // "too many closing" while this crate rendered lex
                    // failures itself; it gained the prefix when the reducer
                    // moved into `rustyfi_syntax::parse_error`, so that the
                    // editor and the terminal say the same sentence about the
                    // same file. Position and severity are unchanged, and
                    // the reason text is still verbatim from the lexer.
                    "message": "parse error: too many closing",
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
    // `rename` is a capability this server does not advertise; a client that
    // asks anyway gets an error rather than a silent hang.
    let (out, code_) = session(&[
        initialize(1),
        json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/rename", "params": {}}),
        shutdown(3),
        exit(),
    ]);
    assert_eq!(out[1]["error"]["code"], code::METHOD_NOT_FOUND);
    assert_eq!(out[2]["id"], 3);
    assert!(out[2]["result"].is_null());
    assert_eq!(code_, 0);
}

/// Every capability the `initialize` reply advertises is answered in one
/// session, and nothing it does not advertise is.
///
/// The per-capability tests further down each drive one request in isolation;
/// this one exists for the failure they cannot see between them — a dispatch
/// arm dropped while the capability that promises it survives, which looks
/// from the editor like a feature that silently does nothing. Reading the
/// advertised set back off the reply rather than restating it is the point:
/// adding a provider without a handler fails here, not in review.
#[test]
fn every_advertised_capability_answers_in_one_session() {
    let uri = "file:///all.saty";
    let src = "let-inline \\emph it = it\nlet greeting = 1\nlet doc = {\\emph{hi}} greeting\n";
    // Formatting asks about a *second* buffer, because the first one is
    // already tidy and a formatter with nothing to do answers `[]` — which is
    // the very shape this test reads as "advertised but does nothing".
    let untidy = "file:///untidy.saty";
    let (out, code_) = session(&[
        initialize(1),
        did_open(uri, src),
        did_open(untidy, "let x = 1   \n\n\n\n\n"),
        at(2, "hover", uri, 2, 12),
        at(3, "definition", uri, 2, 23),
        at(4, "completion", uri, 2, 12),
        document_symbol(5, uri),
        json!({
            "jsonrpc": "2.0", "id": 6, "method": "workspace/symbol",
            "params": { "query": "greeting" },
        }),
        formatting(8, untidy),
        shutdown(7),
        exit(),
    ]);
    assert_eq!(code_, 0);

    // The advertised set, read back rather than restated.
    let caps = out[0]["result"]["capabilities"].as_object().unwrap();
    let advertised: Vec<&str> = caps
        .keys()
        .map(String::as_str)
        .filter(|k| k.ends_with("Provider"))
        .collect();
    assert_eq!(
        advertised,
        [
            "completionProvider",
            "definitionProvider",
            "documentFormattingProvider",
            "documentSymbolProvider",
            "hoverProvider",
            "workspaceSymbolProvider",
        ],
        "a provider was added without a case in this test: {caps:#?}"
    );

    // …and every one of them answered something other than `null`, which is
    // how a lost handler would show up: an advertised capability whose every
    // reply is empty.
    for (id, what) in [
        (2, "hover"),
        (3, "definition"),
        (4, "completion"),
        (5, "documentSymbol"),
        (6, "workspace/symbol"),
        (8, "formatting"),
    ] {
        let r = reply(&out, id);
        assert!(r.get("error").is_none(), "{what} errored: {r:#?}");
        assert!(
            !r["result"].is_null() && r["result"] != json!([]),
            "{what} advertised but answered nothing: {r:#?}"
        );
    }

    // Diagnostics are pushed, not requested, so they are checked by their
    // absence of an error rather than by a reply id: the buffer compiles.
    assert!(diagnostics(&out, 0).is_empty(), "{out:#?}");
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
    assert!(diagnostics(&out, 0).is_empty(), "{out:#?}");

    let broken = "@require: basic\nmodule M = struct\n  val a = = 1\nend\n";
    let (out, _) = session(&[initialize(1), did_open("file:///lib.satyh", broken)]);
    let diags = diagnostics(&out, 0);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0]["range"]["start"], json!({ "line": 2, "character": 10 }));
}

#[test]
fn a_lang_override_is_honoured_over_the_buffers_own_signal() {
    // `let-rec` sniffs as 0.0.6. Forced to 0.1 it must not parse, which is
    // how we know the override reached the analysis rather than being
    // quietly dropped.
    let src = "let-rec f x = x\n";
    let opts = Options {
        lang: Some(RustyfiVersion::V0_1),
        ..Options::default()
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
    // `LineIndex` counts lines itself rather than trusting
    // `rustyfi_syntax::Loc::line`, so the two definitions of "what terminates
    // a line" have to agree. They only differ on CRLF — where a naive
    // implementation counts `\r\n` as two lines and reports every subsequent
    // diagnostic one line too far down — and every fixture in both crates is
    // `\n`-only, so this is the test that pins it.
    let src = "@require: stdjabook\r\nlet x = 1 in\r\nlet y = ] in x\r\n";
    let (out, _) = session(&[initialize(1), did_open("file:///crlf.saty", src)]);
    assert_eq!(
        diagnostics(&out, 0)[0]["range"]["start"],
        json!({ "line": 2, "character": 8 }),
        "CRLF must not shift the line number: {out:#?}"
    );
}

// ---------------------------------------------------------------------------
// Hover, definition and completion, over the wire
// ---------------------------------------------------------------------------
//
// The three interactive requests are *pulled*: the client sends a URI and a
// position and no text, so each of these also exercises the document store —
// a server that forgot to keep the buffer answers `null` to every one of them
// and looks, from the editor, exactly like a server that has no feature.

/// A `textDocument/<method>` request at a zero-based UTF-16 position.
fn at(id: i64, method: &str, uri: &str, line: u32, character: u32) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": format!("textDocument/{method}"),
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
        },
    })
}

/// The `result` of the reply with this id.
fn result(out: &[Value], id: i64) -> &Value {
    out.iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("no reply with id {id}: {out:#?}"))
        .get("result")
        .unwrap_or_else(|| panic!("reply {id} has no result: {out:#?}"))
}

#[test]
fn hover_answers_with_markdown_and_a_range() {
    let src = "let-inline \\emph it = it\nlet doc = {\\emph{hi}}\n";
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///d.saty", src),
        // Line 1, character 12: inside the `\emph` USE.
        at(2, "hover", "file:///d.saty", 1, 12),
    ]);
    let r = result(&out, 2);
    assert_eq!(r["contents"]["kind"], "markdown");
    assert!(
        r["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("bound by `let-inline` on line 1"),
        "{r:#?}"
    );
    assert_eq!(
        r["range"],
        json!({
            "start": { "line": 1, "character": 11 },
            "end":   { "line": 1, "character": 16 },
        }),
    );
}

#[test]
fn hover_on_nothing_answers_null_rather_than_an_error() {
    // A cursor on a keyword names nothing, and `null` is how LSP spells "no
    // result". An error response would make the client log a failure for what
    // is the commonest outcome of all.
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///d.saty", "let x = 1\n"),
        at(2, "hover", "file:///d.saty", 0, 1),
    ]);
    assert_eq!(*result(&out, 2), Value::Null);
    assert!(out.iter().all(|m| m.get("error").is_none()), "{out:#?}");
}

#[test]
fn definition_answers_a_location_in_the_same_document() {
    let src = "let greeting = 1\nlet other = greeting\n";
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///d.saty", src),
        at(2, "definition", "file:///d.saty", 1, 14),
    ]);
    assert_eq!(
        *result(&out, 2),
        json!({
            "uri": "file:///d.saty",
            "range": {
                "start": { "line": 0, "character": 4 },
                "end":   { "line": 0, "character": 12 },
            },
        }),
    );
}

#[test]
fn definition_follows_an_import_header_to_the_file_beside_it() {
    // `@import:` resolves relative to the importing file and needs nothing
    // configured, so this is the cross-file jump that works in any project.
    // It goes through `rustyfi_loader::resolve_import`, the compiler's own.
    let dir = std::env::temp_dir().join(format!("rustyfi-lsp-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dep = dir.join("helper.satyh");
    std::fs::write(&dep, "let helper = 1\n").unwrap();
    let doc = dir.join("doc.saty");
    let uri = format!("file://{}", doc.display());

    let (out, _) = session(&[
        initialize(1),
        did_open(&uri, "@import: helper\nlet x = 1 in x\n"),
        at(2, "definition", &uri, 0, 3),
    ]);
    let r = result(&out, 2);
    assert_eq!(
        r["uri"].as_str().unwrap(),
        format!("file://{}", dep.display()),
        "{r:#?}"
    );
    assert_eq!(r["range"]["start"], json!({ "line": 0, "character": 0 }));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn definition_on_a_require_with_no_library_root_answers_null() {
    // Nothing is configured, so there is nowhere to look — and searching
    // somewhere plausible would open the wrong file.
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///d.saty", "@require: stdjabook\nlet x = 1 in x\n"),
        at(2, "definition", "file:///d.saty", 0, 3),
    ]);
    assert_eq!(*result(&out, 2), Value::Null);
}

#[test]
fn completion_answers_items_with_a_text_edit_over_the_typed_word() {
    let src = "let-inline \\emph it = it\nlet doc = {\\emp";
    let (out, _) = session(&[
        initialize(1),
        did_open("file:///d.saty", src),
        // End of the buffer, just past `\emp`.
        at(2, "completion", "file:///d.saty", 1, 15),
    ]);
    let r = result(&out, 2);
    assert_eq!(r["isIncomplete"], false);
    let items = r["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{r:#?}");
    assert_eq!(items[0]["label"], "\\emph");
    assert_eq!(items[0]["kind"], 3, "CompletionItemKind::Function");
    assert_eq!(items[0]["detail"], "let-inline");
    assert_eq!(
        items[0]["textEdit"],
        json!({
            "range": {
                // The edit covers the `\` too, because the label carries it.
                "start": { "line": 1, "character": 11 },
                "end":   { "line": 1, "character": 15 },
            },
            "newText": "\\emph",
        }),
    );
}

#[test]
fn the_interactive_requests_work_at_a_utf16_column_after_japanese() {
    let src = "let-inline \\ruby it = it\nlet doc = {日本語と\\ruby{かな}}\n";
    let uri = "file:///jp.saty";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, src),
        // `let doc = {` is 11 units and `日本語と` is 4 more, so the command
        // starts at character 15 — 12 bytes further along than that.
        at(2, "hover", uri, 1, 16),
        at(3, "definition", uri, 1, 16),
    ]);
    assert!(
        result(&out, 2)["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Inline command"),
        "{out:#?}"
    );
    assert_eq!(
        result(&out, 3)["range"]["start"],
        json!({ "line": 0, "character": 11 }),
    );
}

#[test]
fn the_interactive_requests_survive_a_half_typed_buffer() {
    // Neither lexes nor parses whole: `{\emp` leaves a command applied to
    // nothing. Everything written before it is still answered about.
    let src = "let alpha = 1\nlet beta = alpha\nlet doc = {\\emp";
    let uri = "file:///half.saty";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, src),
        at(2, "hover", uri, 1, 13),      // the `alpha` mention
        at(3, "definition", uri, 1, 13), // the same
        at(4, "completion", uri, 2, 15), // just past `\emp`
    ]);
    assert!(
        result(&out, 2)["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("bound by `let` on line 1"),
        "{out:#?}"
    );
    assert_eq!(
        result(&out, 3)["range"]["start"],
        json!({ "line": 0, "character": 4 }),
    );
    // No inline command is bound in this buffer, so there is nothing to offer
    // — but the request must still answer a well-formed, empty list.
    assert_eq!(result(&out, 4)["items"], json!([]));
}

#[test]
fn a_request_about_a_closed_document_answers_null() {
    let uri = "file:///d.saty";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, "let x = 1\nlet y = x\n"),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } },
        }),
        at(2, "hover", uri, 1, 9),
    ]);
    assert_eq!(*result(&out, 2), Value::Null);
}

#[test]
fn a_did_change_keeps_the_stored_buffer_in_step() {
    let uri = "file:///d.saty";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, "let alpha = 1\nlet z = alpha\n"),
        did_change(uri, 2, "let renamed = 1\nlet z = renamed\n"),
        at(2, "hover", uri, 1, 10),
    ]);
    let value = result(&out, 2)["contents"]["value"].as_str().unwrap();
    assert!(value.contains("renamed"), "{value}");
    assert!(
        !value.contains("alpha"),
        "the store must not be stale: {value}"
    );
}

#[test]
fn a_ranged_did_change_forgets_the_buffer_rather_than_answering_from_stale_text() {
    // Full sync was advertised; a ranged change means the client is not
    // honouring it, and the server's copy is now known to be out of date.
    // Diagnostics may stay on screen through that (at worst mispositioned),
    // but a hover computed from stale text describes characters that are not
    // there.
    let uri = "file:///d.saty";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, "let alpha = 1\nlet z = alpha\n"),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 4 },
                        "end":   { "line": 0, "character": 9 },
                    },
                    "text": "beta",
                }],
            },
        }),
        at(2, "hover", uri, 1, 10),
    ]);
    assert_eq!(*result(&out, 2), Value::Null);
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

/// The whole exchange, asserted as JSON: this is the wire format an editor
/// reads, and the one place a field-name slip (`selectionRange` written
/// `selection_range`) shows up as a broken outline rather than as a type
/// error.
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

/// A `didChange` replaces the stored buffer, so the outline follows the edits
/// rather than the text the file was opened with. The failure this rules out
/// is a symbol pane that is correct once and then frozen.
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

/// A URI the server was never given — never opened, or closed again — is
/// answered with an empty outline rather than an error, so an editor does not
/// show "request failed" in a pane for a file it has not opened.
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

/// The UTF-16 rule, end to end and on the outline path this time: `title`'s
/// value is 42 bytes of Japanese, and `sub`'s columns are only right if the
/// server counts code units.
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

/// `--lang` pins the generation for the outline just as it does for
/// diagnostics: a 0.1 library asked for as 0.0.6 declares nothing, and the
/// server must not quietly fall back to the reading that works better.
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
            ..Options::default()
        },
    );
    assert_eq!(reply(&pinned, 2)["result"], json!([]));
}

// ---------------------------------------------------------------------------
// textDocument/formatting
// ---------------------------------------------------------------------------

fn formatting(id: i64, uri: &str) -> Value {
    formatting_with(id, uri, json!({ "tabSize": 4, "insertSpaces": true }))
}

/// A formatting request carrying an explicit `FormattingOptions`.
fn formatting_with(id: i64, uri: &str, options: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/formatting",
        "params": { "textDocument": { "uri": uri }, "options": options },
    })
}

/// Apply the reply's edits to `text` the way a client would, so the tests
/// assert the *document the user ends up with* rather than the arithmetic in
/// the range. A wrong range that still reads plausibly in an `assert_eq!` on
/// the JSON would corrupt a buffer here.
fn apply(text: &str, edits: &Value) -> String {
    let index = LineIndex::new(text);
    let mut out = text.to_string();
    // Applied back to front so an earlier edit's offsets stay valid. This
    // server only ever sends one, which the tests below pin separately.
    let mut edits: Vec<&Value> = edits
        .as_array()
        .expect("an array of edits")
        .iter()
        .collect();
    edits.reverse();
    for e in edits {
        let at = |which: &str| {
            let p = &e["range"][which];
            index.offset(Position {
                line: p["line"].as_u64().unwrap() as u32,
                character: p["character"].as_u64().unwrap() as u32,
            })
        };
        out.replace_range(at("start")..at("end"), e["newText"].as_str().unwrap());
    }
    out
}

/// The edit an editor actually receives, asserted as JSON.
///
/// The range is checked literally as well as through [`apply`], because the
/// narrowing in `minimal_edit` is exactly the kind of arithmetic that is right
/// on the buffer and wrong on the wire.
#[test]
fn formatting_answers_one_narrow_edit_rather_than_replacing_the_file() {
    let uri = "file:///untidy.saty";
    // Twenty tidy lines and one untidy one: a whole-document replacement would
    // name a range covering all of them.
    let mut src = String::new();
    for i in 0..20 {
        src.push_str(&format!("let x{i} = {i}\n"));
    }
    src.push_str("let last = 1   \n");
    let (out, _) = session(&[initialize(1), did_open(uri, &src), formatting(2, uri)]);

    let edits = &reply(&out, 2)["result"];
    assert_eq!(edits.as_array().map(Vec::len), Some(1), "{edits:#?}");
    assert_eq!(
        edits[0]["range"],
        json!({
            "start": { "line": 20, "character": 12 },
            "end":   { "line": 20, "character": 15 },
        }),
        "the edit must cover the trailing spaces and nothing else"
    );
    assert_eq!(edits[0]["newText"], "");
    assert_eq!(apply(&src, edits), src.replace("= 1   \n", "= 1\n"));
}

/// An already-formatted buffer answers `[]`, and one that cannot be formatted
/// answers `null`. The difference matters on every save: a format-on-save that
/// saw `null` for a tidy file would report it as unformattable.
#[test]
fn a_tidy_buffer_gets_no_edits_and_a_broken_one_gets_null() {
    let tidy = "file:///tidy.saty";
    let broken = "file:///broken.saty";
    let (out, _) = session(&[
        initialize(1),
        did_open(tidy, "let x = 1\n"),
        // An unterminated inline area: no area map, so no format.
        did_open(broken, "let doc = {hello\n"),
        formatting(2, tidy),
        formatting(3, broken),
        formatting(4, "file:///never-opened.saty"),
    ]);
    assert_eq!(reply(&out, 2)["result"], json!([]));
    assert!(reply(&out, 3)["result"].is_null());
    assert!(reply(&out, 4)["result"].is_null(), "an unknown URI");
    for id in [2, 3, 4] {
        assert!(
            reply(&out, id).get("error").is_none(),
            "declining is not an error: {:#?}",
            reply(&out, id)
        );
    }
}

/// Inline text is content, and the wire path must not be the place that
/// forgets it. This buffer holds doubled spaces and a trailing run inside
/// `{ … }` and neither is touched; the trailing run on the *program* line is.
#[test]
fn formatting_over_the_wire_leaves_prose_alone() {
    let uri = "file:///prose.saty";
    let src = "let doc = {hello  world   \n\n  and  more}   \n";
    let (out, _) = session(&[initialize(1), did_open(uri, src), formatting(2, uri)]);
    let edits = &reply(&out, 2)["result"];
    assert_eq!(
        apply(src, edits),
        "let doc = {hello  world   \n\n  and  more}\n",
        "only the spaces after the closing brace are program text"
    );
}

/// UTF-16 again, on the formatting path: the edit's range is only right if the
/// server counts code units, and Japanese before the edit is what separates
/// that from counting bytes.
#[test]
fn formatting_ranges_are_utf16_over_the_wire() {
    let uri = "file:///jp.saty";
    let src = "let title = `日本語のタイトル`   \n";
    assert_eq!(src.find("   \n"), Some(38), "byte offset, for contrast");
    let (out, _) = session(&[initialize(1), did_open(uri, src), formatting(2, uri)]);
    let edits = &reply(&out, 2)["result"];
    assert_eq!(
        edits[0]["range"]["start"],
        json!({ "line": 0, "character": 22 }),
        "22 UTF-16 units, not 38 bytes"
    );
    assert_eq!(apply(src, edits), "let title = `日本語のタイトル`\n");
}

/// The client's `FormattingOptions` reach the formatter. `insertSpaces: false`
/// is the one that is easy to accept and then ignore, because ignoring it
/// still produces valid output — just not the output that was asked for.
#[test]
fn the_clients_formatting_options_are_honoured() {
    let uri = "file:///tabs.saty";
    let src = "let f x =\n\tx\n";
    let msgs = |options: Value| {
        [
            initialize(1),
            did_open(uri, src),
            formatting_with(2, uri, options),
        ]
    };

    let (spaces, _) = session(&msgs(json!({ "tabSize": 2, "insertSpaces": true })));
    assert_eq!(apply(src, &reply(&spaces, 2)["result"]), "let f x =\n  x\n");

    let (tabs, _) = session(&msgs(json!({ "tabSize": 2, "insertSpaces": false })));
    assert_eq!(reply(&tabs, 2)["result"], json!([]), "tabs were asked for");

    // The optional members turn individual rules off.
    let uri2 = "file:///opts.saty";
    let (kept, _) = session(&[
        initialize(1),
        did_open(uri2, "let x = 1   \n"),
        formatting_with(
            2,
            uri2,
            json!({ "tabSize": 4, "insertSpaces": true, "trimTrailingWhitespace": false }),
        ),
    ]);
    assert_eq!(reply(&kept, 2)["result"], json!([]));
}

/// `--lang` pins the generation for formatting too. `@stage:` is a header 0.1
/// deleted outright, so the pin is visible as a decline rather than as a
/// different tidy-up.
#[test]
fn formatting_obeys_an_explicit_lang() {
    let uri = "file:///lib.satyh";
    let src = "@stage: 1\nlet x = 1   \n";
    let msgs = [initialize(1), did_open(uri, src), formatting(2, uri)];

    let (auto, _) = session(&msgs);
    assert_eq!(
        apply(src, &reply(&auto, 2)["result"]),
        "@stage: 1\nlet x = 1\n"
    );

    let (pinned, _) = session_with(
        &msgs,
        Options {
            lang: Some(RustyfiVersion::V0_1),
            ..Options::default()
        },
    );
    assert!(
        reply(&pinned, 2)["result"].is_null(),
        "0.1 has no `@stage:` header, so this buffer does not lex as 0.1"
    );
}

/// Formatting reads the stored buffer, so it follows `didChange` rather than
/// the text the file was opened with.
#[test]
fn formatting_follows_did_change() {
    let uri = "file:///doc.saty";
    let edited = "let after = 1   \n";
    let (out, _) = session(&[
        initialize(1),
        did_open(uri, "let before = 1\n"),
        did_change(uri, 2, edited),
        formatting(3, uri),
    ]);
    assert_eq!(apply(edited, &reply(&out, 3)["result"]), "let after = 1\n");
}
