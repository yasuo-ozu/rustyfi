//! `workspace/symbol` driven end to end over a real directory tree.
//!
//! Kept apart from `server_stdio.rs` because it is the one part of the server
//! that needs a filesystem: every test here builds a throwaway project under
//! the system temp directory, points `initialize` at it, and asserts the
//! matches that come back.
//!
//! No `tempfile` dependency — the crate graph is deliberately small, and a
//! directory named after the process and a counter is enough for a test that
//! removes it again.

use rustyfi_lsp::jsonrpc;
use rustyfi_lsp::server::{self, Options};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A directory that deletes itself.
struct Project(PathBuf);

impl Project {
    /// Build a project from `(relative path, contents)` pairs.
    fn new(name: &str, files: &[(&str, &str)]) -> Project {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("rustyfi-lsp-{name}-{}-{nanos}", std::process::id()));
        for (rel, body) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body).unwrap();
        }
        Project(root)
    }

    fn uri(&self) -> String {
        uri_of(&self.0)
    }

    fn file_uri(&self, rel: &str) -> String {
        uri_of(&self.0.join(rel))
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The same escaping `workspace.rs` does, written independently here so a
/// change to one is not silently mirrored in the other.
fn uri_of(p: &Path) -> String {
    let mut out = String::from("file://");
    for b in p.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn run(msgs: &[Value]) -> Vec<Value> {
    let mut input = Vec::new();
    for m in msgs {
        jsonrpc::write_message(&mut input, m).unwrap();
    }
    let mut raw = Vec::new();
    server::run(
        &mut std::io::Cursor::new(input),
        &mut raw,
        Options::default(),
    )
    .expect("the server must not fail on I/O");

    let mut cursor = std::io::Cursor::new(raw);
    let mut out = Vec::new();
    while let Some(v) = jsonrpc::read_message(&mut cursor).unwrap() {
        out.push(v);
    }
    out
}

fn initialize(root: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "processId": null, "rootUri": root, "capabilities": {} },
    })
}

fn symbol_query(id: i64, query: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "workspace/symbol",
        "params": { "query": query },
    })
}

fn reply(out: &[Value], id: i64) -> &Value {
    out.iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("no reply with id {id}: {out:#?}"))
}

/// `name@uri-basename` for each match, so an assertion reads as a list.
fn hits(result: &Value) -> Vec<String> {
    result
        .as_array()
        .expect("an array of matches")
        .iter()
        .map(|m| {
            let uri = m["location"]["uri"].as_str().unwrap();
            let base = uri.rsplit('/').next().unwrap();
            format!("{}@{base}", m["name"].as_str().unwrap())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const LIB_006: &str = "@require: list\n\nlet alpha = 1\nlet alphabet x = x\n";
const LIB_01: &str = "module Beta = struct\n  val beta-one = 1\n  type t = int\nend\n";

/// The basic query: a substring, matched case-insensitively, across two files
/// of two different generations, one of them in a subdirectory.
#[test]
fn a_query_matches_across_the_whole_project() {
    let p = Project::new("query", &[("a.satyh", LIB_006), ("sub/b.satyh", LIB_01)]);
    let out = run(&[initialize(&p.uri()), symbol_query(2, "ALPHA")]);

    let mut got = hits(&reply(&out, 2)["result"]);
    got.sort();
    assert_eq!(got, ["alpha@a.satyh", "alphabet@a.satyh"]);
}

/// Both generations are read with their own grammar, from the same query —
/// the 0.1 library here is signal-free (`module M = struct …`), so this is
/// also the version re-check working through the workspace path.
#[test]
fn both_generations_contribute() {
    let p = Project::new("gens", &[("a.satyh", LIB_006), ("sub/b.satyh", LIB_01)]);
    let out = run(&[initialize(&p.uri()), symbol_query(2, "beta")]);

    let mut got = hits(&reply(&out, 2)["result"]);
    got.sort();
    assert_eq!(got, ["Beta@b.satyh", "beta-one@b.satyh"]);
}

/// A nested symbol carries the name of the declaration it lives in, which is
/// what an editor shows in the second column of its "go to symbol in
/// workspace" list.
#[test]
fn a_nested_match_names_its_container() {
    let p = Project::new("container", &[("b.satyh", LIB_01)]);
    let out = run(&[initialize(&p.uri()), symbol_query(2, "beta-one")]);

    let result = &reply(&out, 2)["result"];
    assert_eq!(result[0]["containerName"], "Beta");
    // A top-level symbol has none, rather than an empty string.
    let out = run(&[initialize(&p.uri()), symbol_query(2, "Beta")]);
    let top = &reply(&out, 2)["result"][0];
    assert_eq!(top["name"], "Beta");
    assert!(top.get("containerName").is_none(), "{top}");
}

/// The location is a real, usable jump target: the declaration's own range,
/// in the same UTF-16 coordinates everything else in this crate uses.
#[test]
fn a_match_points_at_the_declaration() {
    let p = Project::new("location", &[("a.satyh", LIB_006)]);
    let out = run(&[initialize(&p.uri()), symbol_query(2, "alphabet")]);

    let m = &reply(&out, 2)["result"][0];
    assert_eq!(m["location"]["uri"], p.file_uri("a.satyh"));
    assert_eq!(
        m["location"]["range"],
        json!({
            "start": { "line": 3, "character": 0 },
            "end":   { "line": 3, "character": 18 },
        }),
    );
    // 12 = Function: `alphabet` takes a parameter, `alpha` does not.
    assert_eq!(m["kind"], 12);
}

/// An **unsaved buffer wins over the file on disk**, so a name you have just
/// typed is findable before you save — and one you have just deleted stops
/// being found.
#[test]
fn an_open_buffer_shadows_the_file_on_disk() {
    let p = Project::new("overlay", &[("a.satyh", LIB_006)]);
    let uri = p.file_uri("a.satyh");
    let out = run(&[
        initialize(&p.uri()),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": uri, "languageId": "satysfi", "version": 1,
                "text": "let renamed = 1\n",
            }},
        }),
        symbol_query(2, "alpha"),
        symbol_query(3, "renamed"),
    ]);

    assert_eq!(hits(&reply(&out, 2)["result"]), Vec::<String>::new());
    assert_eq!(hits(&reply(&out, 3)["result"]), ["renamed@a.satyh"]);
}

/// An empty query is what a client sends when its box is empty; it lists
/// everything rather than erroring or returning nothing.
#[test]
fn an_empty_query_lists_everything() {
    let p = Project::new("empty", &[("a.satyh", LIB_006), ("sub/b.satyh", LIB_01)]);
    let out = run(&[initialize(&p.uri()), symbol_query(2, "")]);

    let got = hits(&reply(&out, 2)["result"]);
    assert!(got.len() >= 6, "{got:?}");
    assert!(got.contains(&"list@a.satyh".to_string()), "{got:?}");
}

/// A session with no workspace at all — a client attached to one file — is
/// answered from the open buffers rather than refused.
#[test]
fn a_session_with_no_root_still_searches_its_open_buffers() {
    let out = run(&[
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "processId": null, "rootUri": null, "capabilities": {} },
        }),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": "file:///tmp/detached.satyh", "languageId": "satysfi",
                "version": 1, "text": "let solo = 1\n",
            }},
        }),
        symbol_query(2, "solo"),
    ]);
    assert_eq!(hits(&reply(&out, 2)["result"]), ["solo@detached.satyh"]);
}

/// Two identical queries answer identically. `read_dir` order is not stable
/// across filesystems, and a result list that reshuffles itself between
/// keystrokes moves the entry under the user's cursor.
#[test]
fn repeating_a_query_gives_the_same_order() {
    let p = Project::new(
        "stable",
        &[
            ("a.satyh", LIB_006),
            ("b.satyh", LIB_006),
            ("sub/c.satyh", LIB_01),
        ],
    );
    let out = run(&[
        initialize(&p.uri()),
        symbol_query(2, "a"),
        symbol_query(3, "a"),
    ]);
    assert_eq!(
        reply(&out, 2)["result"],
        reply(&out, 3)["result"],
        "the second query must not reorder the first's answer"
    );
}

/// A directory whose name needs escaping round-trips: the URI in the reply
/// has to be one the client can turn back into this path.
#[test]
fn a_path_needing_escapes_round_trips() {
    let p = Project::new("escaped", &[("な ま/a.satyh", LIB_006)]);
    let out = run(&[initialize(&p.uri()), symbol_query(2, "alpha")]);

    let uri = reply(&out, 2)["result"][0]["location"]["uri"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(uri.contains("%20"), "the space must be escaped: {uri}");
    assert_eq!(uri, p.file_uri("な ま/a.satyh"));
}
