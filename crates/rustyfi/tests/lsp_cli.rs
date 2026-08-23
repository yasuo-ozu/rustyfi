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

/// The whole-program tier, through the real binary: a document whose every
/// name comes from a package it `@require:`s, and the same document with one
/// type error added.
///
/// This is the pair that matters. The clean half proves the tier does not
/// invent diagnostics out of a resolved program (the failure mode that makes
/// a language server worse than none); the broken half proves it finds a real
/// error and puts it on the right characters. Neither is possible at the
/// parse tier — `\emph` and `document` are not in the buffer.
#[test]
fn the_whole_program_tier_reports_a_type_error_and_stays_quiet_otherwise() {
    let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let lib_root = repo.join("lib-rustyfi");
    // A path inside the repository that does not exist: an unsaved buffer,
    // which is what an editor hands over most of the time. Its directory is
    // real, which is all `@import:` resolution needs.
    let uri = format!(
        "file://{}",
        repo.join("layout-tests/unsaved-lsp-cli.saty").display()
    );

    let clean = "@require: stdja-mini\n\
                 let author-name = `yasuo`\n\
                 in\n\
                 document (| title = {T}; author = {A} |) '<\n\
                 \x20 +p { Hello, \\emph{world}. }\n\
                 >\n";
    let (out, _) = lsp_session(
        &["--lib-root", lib_root.to_str().unwrap()],
        &[initialize(1), did_open(&uri, clean)],
    );
    assert_eq!(
        out[1]["params"]["diagnostics"],
        json!([]),
        "a document that compiles must publish an EMPTY list: {out:#?}"
    );

    // `succ` is `int -> int` and `s` is a string. The span the checker
    // records for a bad application is the function's, and `succ` on line 3
    // (zero-based) starts at character 8.
    let broken = "@require: stdja-mini\n\
                  let succ n = n + 1\n\
                  let s = `oops`\n\
                  let m = succ s\n\
                  in\n\
                  document (| title = {T}; author = {A} |) '<\n\
                  \x20 +p { Hello. }\n\
                  >\n";
    let (out, _) = lsp_session(
        &["--lib-root", lib_root.to_str().unwrap()],
        &[initialize(1), did_open(&uri, broken)],
    );
    let diags = out[1]["params"]["diagnostics"]
        .as_array()
        .expect("a diagnostics array");
    assert_eq!(diags.len(), 1, "{out:#?}");
    assert_eq!(
        diags[0]["range"],
        json!({
            "start": { "line": 3, "character": 8 },
            "end":   { "line": 3, "character": 12 },
        }),
    );
    assert_eq!(diags[0]["severity"], 1);
    assert_eq!(diags[0]["source"], "rustyfi");
    let message = diags[0]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("int") && message.contains("string"),
        "the message should name both types: {message}"
    );
}

/// The library root can come from the client's `initializationOptions`
/// instead of the command line — the shape an editor plugin configures.
#[test]
fn a_lib_root_from_initialization_options_reaches_the_analysis() {
    let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let lib_root = repo.join("lib-rustyfi");
    let uri = format!(
        "file://{}",
        repo.join("layout-tests/unsaved-lsp-cli-3.saty").display()
    );
    // Unbound without `stdja-mini`, resolved with it: a diagnostic here means
    // the root never arrived.
    let src = "@require: stdja-mini\n\
               document (| title = {T}; author = {A} |) '<\n\
               \x20 +p { \\emph{Hello}. }\n\
               >\n";
    let (out, _) = lsp_session(
        &[],
        &[
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "initializationOptions": { "libRoot": lib_root.to_str().unwrap() }},
            }),
            did_open(&uri, src),
        ],
    );
    assert_eq!(out[1]["params"]["diagnostics"], json!([]), "{out:#?}");

    // The control: the same buffer with a root that has no packages in it at
    // all still publishes nothing — degradation, not a wall of red — which is
    // what makes the assertion above about the ROOT rather than about the
    // tier being off.
    let (out, _) = lsp_session(
        &[],
        &[
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "initializationOptions": { "libRoot": "/nonexistent-root" }},
            }),
            did_open(&uri, "@require: stdja-mini\nlet x = 1 in x\n"),
        ],
    );
    assert_eq!(out[1]["params"]["diagnostics"], json!([]));
}

/// `--no-typecheck` puts the server back on the parse tier, and the parse
/// tier has nothing to say about a type error.
#[test]
fn no_typecheck_leaves_only_the_parse_tier() {
    let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let lib_root = repo.join("lib-rustyfi");
    let uri = format!(
        "file://{}",
        repo.join("layout-tests/unsaved-lsp-cli-2.saty").display()
    );
    let broken = "@require: stdja-mini\n\
                  let succ n = n + 1\n\
                  let s = `oops`\n\
                  let m = succ s\n\
                  in\n\
                  document (| title = {T}; author = {A} |) '<\n\
                  \x20 +p { Hello. }\n\
                  >\n";
    let (out, _) = lsp_session(
        &[
            "--no-typecheck",
            "--lib-root",
            lib_root.to_str().unwrap(),
        ],
        &[initialize(1), did_open(&uri, broken)],
    );
    assert_eq!(out[1]["params"]["diagnostics"], json!([]));
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
