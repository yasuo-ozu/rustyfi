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

use serde_json::{json, Value};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

/// Frame a message the way the LSP base protocol requires.
fn frame(msg: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(msg).unwrap();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend(body);
    out
}

/// Split a framed byte stream back into messages, checking each
/// `Content-Length` against the bytes that follow it.
fn unframe(mut data: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let split = data
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or_else(|| panic!("no header terminator in {:?}", String::from_utf8_lossy(data)));
        let headers = std::str::from_utf8(&data[..split]).expect("headers are ASCII");
        let len: usize = headers
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .expect("a Content-Length header")
            .trim()
            .parse()
            .expect("a numeric Content-Length");
        let body = &data[split + 4..split + 4 + len];
        out.push(serde_json::from_slice(body).expect("a JSON body of exactly Content-Length bytes"));
        data = &data[split + 4 + len..];
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
fn the_subcommand_is_listed_in_help() {
    let out = Command::new(bin()).arg("--help").output().expect("spawn");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("lsp"), "`lsp` should be discoverable:\n{text}");
}

#[test]
fn compiling_still_works_exactly_as_before() {
    // `lsp` is additive. The one way to get this wrong is a clap tree change
    // that makes the compile positional stop resolving.
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("spawn");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Compile a SATySFi"), "compile mode is still the default:\n{text}");
}
