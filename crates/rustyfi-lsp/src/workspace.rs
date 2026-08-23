//! `workspace/symbol`: the same outline extraction, across a whole project.
//!
//! This is the one part of the server that touches the filesystem, and it is
//! kept in its own module for exactly that reason — [`crate::symbols`] and
//! [`crate::analyze`] stay pure and wasm-safe, and everything here is behind
//! the `server` feature.
//!
//! # What it does
//!
//! Walks the workspace folders the client named at `initialize`, extracts
//! [`crate::Symbol`]s from every SATySFi file it finds, flattens the trees and
//! filters by the query. An **open buffer wins over the file on disk**, so a
//! rename you have not saved yet is findable under its new name.
//!
//! # Why it caches
//!
//! An editor sends `workspace/symbol` on every keystroke of the query box.
//! Extracting the bundled corpus costs about a second, which would make the
//! feature unusable and the editor unresponsive with it — so results are kept
//! per file and re-derived only when the file's length or modification time
//! changes. The first query pays for the scan; the rest are a filter over
//! memory.
//!
//! # Bounds
//!
//! [`MAX_FILES`] and [`MAX_RESULTS`] are hard caps. A language server pointed
//! at a home directory by an over-eager client must degrade to "incomplete
//! answers" rather than to "reads a million files"; neither limit is
//! reachable by a real SATySFi project (the largest thing in this repository
//! is 247 files).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{json, Value};

use crate::{RustyfiVersion, Symbol};

/// How many files one scan will read. See the module comment.
const MAX_FILES: usize = 4_000;

/// How many symbols one query will answer with.
const MAX_RESULTS: usize = 512;

/// Directory names never descended into: build output and version-control
/// metadata, plus anything hidden. `target` is Rust's, `_build` is
/// Satyrographos'.
const SKIP_DIRS: &[&str] = &["target", "_build", "node_modules"];

/// The workspace roots, and the outline of every file under them.
#[derive(Default)]
pub(crate) struct Workspace {
    roots: Vec<PathBuf>,
    cache: HashMap<PathBuf, Entry>,
}

/// One file's outline, and what it was derived from.
struct Entry {
    stamp: Stamp,
    symbols: Vec<Symbol>,
}

/// Cheap evidence that a file has not changed. Not a hash: hashing every file
/// on every keystroke is most of the cost re-extraction would have been.
#[derive(PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl Workspace {
    /// Read the roots out of an `initialize` request.
    ///
    /// `workspaceFolders` is the modern spelling and may name several;
    /// `rootUri` is the older single-folder one, and `rootPath` older still.
    /// All three are optional — a client may attach to a single file with no
    /// workspace at all, and then there is nothing to search but the open
    /// buffers, which is handled without any roots.
    pub(crate) fn absorb_initialize(&mut self, params: &Value) {
        if let Some(folders) = params.get("workspaceFolders").and_then(Value::as_array) {
            for f in folders {
                if let Some(p) = f.get("uri").and_then(Value::as_str).and_then(uri_to_path) {
                    self.roots.push(p);
                }
            }
        }
        if let Some(p) = params
            .get("rootUri")
            .and_then(Value::as_str)
            .and_then(uri_to_path)
        {
            self.roots.push(p);
        }
        if let Some(p) = params.get("rootPath").and_then(Value::as_str) {
            self.roots.push(PathBuf::from(p));
        }
        self.roots.sort();
        self.roots.dedup();
    }

    /// Answer one `workspace/symbol`.
    ///
    /// `open` is the server's buffer store; those texts shadow whatever is on
    /// disk at the same path.
    pub(crate) fn query(
        &mut self,
        query: &str,
        open: &HashMap<String, String>,
        lang: Option<RustyfiVersion>,
    ) -> Vec<Value> {
        self.refresh(open, lang);

        let needle = query.to_lowercase();
        let mut out = Vec::new();
        // Sorted, so the answer to the same query is the same list in the
        // same order twice running — `read_dir` order is not stable across
        // filesystems and an outline that reshuffles itself is disorienting.
        let mut paths: Vec<&PathBuf> = self.cache.keys().collect();
        paths.sort();
        for path in paths {
            let uri = path_to_uri(path);
            collect(&self.cache[path].symbols, None, &needle, &uri, &mut out);
            if out.len() >= MAX_RESULTS {
                out.truncate(MAX_RESULTS);
                break;
            }
        }
        out
    }

    /// Bring the cache in line with the roots and the open buffers.
    fn refresh(&mut self, open: &HashMap<String, String>, lang: Option<RustyfiVersion>) {
        let mut seen: Vec<PathBuf> = Vec::new();
        for root in &self.roots {
            walk(root, &mut seen);
            if seen.len() >= MAX_FILES {
                seen.truncate(MAX_FILES);
                break;
            }
        }
        // An open buffer for a file outside every root is still worth
        // searching — the user is looking at it.
        for uri in open.keys() {
            if let Some(p) = uri_to_path(uri) {
                seen.push(p);
            }
        }
        seen.sort();
        seen.dedup();

        // Files that went away take their symbols with them.
        self.cache.retain(|p, _| seen.binary_search(p).is_ok());

        for path in seen {
            let overlay = open
                .get(&path_to_uri(&path))
                .or_else(|| open.get(path.to_str().unwrap_or_default()));
            let stamp = match overlay {
                // A buffer's stamp is its own length with no timestamp, so a
                // buffer and the file it shadows never compare equal and an
                // edit always re-extracts.
                Some(text) => Stamp {
                    len: text.len() as u64,
                    modified: None,
                },
                None => match stamp_of(&path) {
                    Some(s) => s,
                    None => continue,
                },
            };
            if self.cache.get(&path).is_some_and(|e| e.stamp == stamp) {
                continue;
            }
            let text = match overlay {
                Some(text) => text.clone(),
                None => match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(_) => continue,
                },
            };
            let symbols = match lang {
                Some(lang) => crate::document_symbols(&text, lang),
                None => crate::document_symbols_auto(&text),
            };
            self.cache.insert(path, Entry { stamp, symbols });
        }
    }
}

/// Flatten one file's tree into `SymbolInformation`-shaped matches.
///
/// The wire shape is `{name, kind, location, containerName}`, which is both a
/// `SymbolInformation` and a `WorkspaceSymbol` with a full `Location` — so it
/// works with a client of either protocol vintage.
fn collect(
    syms: &[Symbol],
    container: Option<&str>,
    needle: &str,
    uri: &str,
    out: &mut Vec<Value>,
) {
    for s in syms {
        if out.len() >= MAX_RESULTS {
            return;
        }
        if needle.is_empty() || s.name.to_lowercase().contains(needle) {
            let mut entry = json!({
                "name": s.name,
                "kind": s.kind.code(),
                "location": {
                    "uri": uri,
                    "range": {
                        "start": {
                            "line": s.range.start.line,
                            "character": s.range.start.character,
                        },
                        "end": {
                            "line": s.range.end.line,
                            "character": s.range.end.character,
                        },
                    },
                },
            });
            if let Some(c) = container {
                entry["containerName"] = Value::String(c.to_string());
            }
            out.push(entry);
        }
        collect(&s.children, Some(&s.name), needle, uri, out);
    }
}

/// Every SATySFi source file under `dir`, recursively, appended to `out`.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        // `file_type` rather than `path.is_dir()`: the latter follows
        // symlinks, and a symlink loop in a workspace would not terminate.
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(t) if t.is_file() => {
                if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("saty" | "satyh" | "satyg")
                ) {
                    out.push(path);
                }
            }
            _ => {}
        }
        if out.len() >= MAX_FILES {
            return;
        }
    }
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let md = std::fs::metadata(path).ok()?;
    Some(Stamp {
        len: md.len(),
        modified: md.modified().ok(),
    })
}

/// `file:///a/b%20c.saty` → `/a/b c.saty`.
///
/// Only the `file` scheme: nothing else names something this server can read.
/// A `file://host/…` URI with a non-empty authority is declined rather than
/// guessed at.
pub(crate) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///path` leaves `/path`; `file://host/path` leaves `host/path`,
    // which is a remote share this server cannot read.
    if !rest.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(rest)?;
    // `file:///C:/x` on Windows: the leading slash is not part of the path.
    #[cfg(windows)]
    let decoded = decoded
        .strip_prefix('/')
        .filter(|r| r.as_bytes().get(1) == Some(&b':'))
        .map(str::to_string)
        .unwrap_or(decoded);
    Some(PathBuf::from(decoded))
}

/// `/a/b c.saty` → `file:///a/b%20c.saty`.
pub(crate) fn path_to_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !raw.starts_with('/') {
        out.push('/');
    }
    for b in raw.bytes() {
        // RFC 3986's unreserved set, plus the separators a path needs to keep
        // literal. Everything else — spaces, `#`, `?`, and every non-ASCII
        // byte of a Japanese directory name — is escaped.
        if b.is_ascii_alphanumeric() || b"-_.~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-decoding, rejecting a sequence that is not valid UTF-8 rather than
/// replacing it — a path this server cannot name is better skipped than
/// silently pointed somewhere else.
fn percent_decode(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_round_trips_through_a_path() {
        for raw in [
            "/a/b.saty",
            "/a b/c.saty",
            "/日本語/x.satyh",
            "/a~b/c-d.satyg",
        ] {
            let uri = path_to_uri(Path::new(raw));
            assert!(uri.starts_with("file:///"), "{uri}");
            assert_eq!(uri_to_path(&uri).as_deref(), Some(Path::new(raw)), "{uri}");
        }
    }

    #[test]
    fn a_space_and_a_kanji_are_escaped_on_the_way_out() {
        assert_eq!(path_to_uri(Path::new("/a b.saty")), "file:///a%20b.saty");
        assert_eq!(path_to_uri(Path::new("/あ.saty")), "file:///%E3%81%82.saty");
    }

    #[test]
    fn a_non_file_scheme_and_a_remote_share_are_declined() {
        assert_eq!(uri_to_path("untitled:Untitled-1"), None);
        assert_eq!(uri_to_path("https://example.com/x.saty"), None);
        assert_eq!(uri_to_path("file://host/share/x.saty"), None);
    }
}
