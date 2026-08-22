//! `rustyfi lsp`, driven as a real subprocess over real pipes.
//!
//! `rustyfi-lsp`'s own `tests/server_stdio.rs` already covers the protocol in
//! process. What only a subprocess can prove is the wiring around it: that
//! the subcommand exists, that it reads *stdin* and writes *stdout* (and not,
//! say, a buffered stdout that never flushes before the process exits), that
//! nothing else in the binary prints to stdout and corrupts the stream, and
//! that the exit code the specification asks for actually reaches the shell.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use rustyfi_lsp::jsonrpc;
use serde_json::{json, Value};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

/// Frame a message, and read a framed stream back, through the server's own
/// base-protocol codec.
///
/// A hand-rolled decoder here would be a second, weaker implementation to
/// keep in step — weaker because it would inevitably be spelled
/// `strip_prefix("Content-Length: ")`, where `read_message` is
/// case-insensitive and whitespace-tolerant per RFC 7230. Using the real one
/// also means this test fails if the *writer* emits a byte count that does
/// not match its body, which is the framing bug worth catching.
fn frame(msg: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    jsonrpc::write_message(&mut out, msg).expect("writing to a Vec cannot fail");
    out
}

fn unframe(data: &[u8]) -> Vec<Value> {
    let mut cursor = std::io::Cursor::new(data);
    let mut out = Vec::new();
    while let Some(v) = jsonrpc::read_message(&mut cursor).expect("a well-framed stream") {
        out.push(v);
    }
    out
}

/// Run `rustyfi lsp <extra_args>`, feed it `msgs`, and return
/// `(messages, exit code)`.
fn lsp_session(extra_args: &[&str], msgs: &[Value]) -> (Vec<Value>, i32) {
    let mut child = Command::new(bin())
        .arg("lsp")
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn `rustyfi lsp`");
    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        for m in msgs {
            stdin.write_all(&frame(m)).expect("write to the server");
        }
        // Dropping stdin closes the pipe, which is also how an editor that
        // never sends `exit` looks to the server.
    }
    let out = child.wait_with_output().expect("wait for `rustyfi lsp`");
    assert!(
        out.stderr.is_empty(),
        "the server should be silent on stderr in a normal session, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (unframe(&out.stdout), out.status.code().expect("an exit code"))
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

#[test]
fn the_subcommand_speaks_lsp_on_stdio_and_reports_a_diagnostic() {
    // Japanese before the error on the SAME line, so this also proves the
    // UTF-16 column reaches a real client through a real pipe: `]` is at byte
    // 8 of line 2 here, but the literal on line 1 is 15 bytes of kana that a
    // whole-file byte offset would have leaked forward.
    let src = "@require: stdjabook\nlet x = `こんにちは` in\nlet y = ] in y\n";
    let (out, exit) = lsp_session(
        &[],
        &[
            initialize(1),
            json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
            did_open("file:///doc.saty", src),
            json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
            json!({"jsonrpc": "2.0", "method": "exit"}),
        ],
    );

    assert_eq!(exit, 0, "shutdown then exit is a clean exit");
    assert_eq!(out.len(), 3, "{out:#?}");

    assert_eq!(out[0]["id"], 1);
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "rustyfi-lsp");
    assert_eq!(out[0]["result"]["capabilities"]["positionEncoding"], "utf-16");

    assert_eq!(out[1]["method"], "textDocument/publishDiagnostics");
    assert_eq!(out[1]["params"]["uri"], "file:///doc.saty");
    assert_eq!(
        out[1]["params"]["diagnostics"][0]["range"],
        json!({
            "start": { "line": 2, "character": 8 },
            "end":   { "line": 2, "character": 9 },
        }),
    );
    assert_eq!(out[1]["params"]["diagnostics"][0]["severity"], 1);

    assert_eq!(out[2]["id"], 2);
    assert!(out[2]["result"].is_null());
}

#[test]
fn exit_without_shutdown_is_exit_code_one() {
    let (_, exit) = lsp_session(
        &[],
        &[initialize(1), json!({"jsonrpc": "2.0", "method": "exit"})],
    );
    assert_eq!(exit, 1);
}

#[test]
fn a_lang_flag_reaches_the_analysis() {
    // `let-rec` is 0.0.6-only; forcing 0.1 must produce a diagnostic, and the
    // same buffer with no flag must not. If `--lang` were dropped on the
    // floor the two would agree.
    let src = "let-rec f x = x\n";
    let (forced, _) = lsp_session(&["--lang", "0.1"], &[initialize(1), did_open("file:///a.saty", src)]);
    let (detected, _) = lsp_session(&[], &[initialize(1), did_open("file:///a.saty", src)]);

    assert!(
        !forced[1]["params"]["diagnostics"].as_array().unwrap().is_empty(),
        "--lang 0.1 must be honoured: {forced:#?}"
    );
    assert_eq!(detected[1]["params"]["diagnostics"], json!([]));
}

#[test]
fn a_bad_lang_flag_is_a_usage_error_not_a_broken_session() {
    let out = Command::new(bin())
        .args(["lsp", "--lang", "9.9"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "nothing may reach the protocol channel");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--lang"));
}

#[test]
fn the_subcommand_is_listed_in_help_without_displacing_compile_mode() {
    let out = Command::new(bin()).arg("--help").output().expect("spawn");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("lsp"), "`lsp` should be discoverable:\n{text}");
    assert!(
        text.contains("Compile a SATySFi"),
        "compile mode is still the default personality:\n{text}"
    );
}

#[test]
fn a_bare_path_still_reaches_compile_mode_rather_than_the_new_subcommand() {
    // `lsp` is additive, and the way to break that is a clap tree change that
    // makes the compile positional stop resolving. A path that does not exist
    // must therefore fail as a *compile* error (exit 1), not as a clap usage
    // error (exit 2) — the latter would mean the positional never bound.
    let out = Command::new(bin())
        .arg("/nonexistent/definitely-not-here.saty")
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected a compile failure, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("definitely-not-here.saty"));
}
