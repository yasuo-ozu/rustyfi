//! Saphe phase 7d slice S1: the real `http` feature transport, tested against
//! a mocked local server (a hand-rolled `TcpListener` loopback, no real
//! internet) exercising the actual `ureq` + rustls client in `registry.rs`'s
//! `http` module. The index stays a plain local directory; only the tarball
//! fetch is new-network.
//!
//! Runs only when the `http` feature is enabled, which is now the crate's
//! default; `cargo test --no-default-features` skips this file entirely,
//! while `tests/registry.rs`'s `file://`/git coverage keeps running either
//! way.

#![cfg(feature = "http")]

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rustyfi_satyrographos::{self as sg, InstallOptions, RegistryKind, RegistryOptions, RootOptions};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satyrographos-http-registry-{tag}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&p, content).expect("write fixture file");
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Build a `.tar.gz` of a manifest package `<name>` at `<tmp>/tarballs/
/// <name>-<version>.tar.gz`, returning `(bytes, lowercase-hex-sha256)` — the
/// bytes are what the mock server serves, so no on-disk `file://` URL is
/// involved at all (unlike `tests/registry.rs`).
fn make_tarball_bytes(tmp: &TempDir, name: &str, version: &str, body: &str) -> (Vec<u8>, String) {
    let src = tmp.path().join(format!("src/{name}-{version}"));
    fs::create_dir_all(src.join("packages")).unwrap();
    fs::write(
        src.join("rustyfi-package.toml"),
        format!(
            "[package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             rustyfi-version-compat = \">=0.0.6, <0.1\"\n\
             \n\
             [[files]]\n\
             kind = \"package-dir\"\n\
             src = \"packages\"\n"
        ),
    )
    .unwrap();
    fs::write(src.join(format!("packages/{name}.satyh")), body).unwrap();

    let tarball = tmp.path().join(format!("tarballs/{name}-{version}.tar.gz"));
    fs::create_dir_all(tarball.parent().unwrap()).unwrap();
    let status = Command::new("tar")
        .args([
            "-czf",
            tarball.to_str().unwrap(),
            "-C",
            src.to_str().unwrap(),
            ".",
        ])
        .status()
        .expect("run tar");
    assert!(status.success(), "tar failed");

    let bytes = fs::read(&tarball).expect("read tarball bytes");
    let sha = sha256_of_bytes(&bytes);
    (bytes, sha)
}

fn sha256_of_bytes(bytes: &[u8]) -> String {
    // Shell out to sha256sum via stdin, mirroring tests/registry.rs's
    // sha256_of (which hashes a file) — no need for a new hashing dep.
    use std::process::Stdio;
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn sha256sum");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(bytes)
        .expect("write to sha256sum stdin");
    let out = child.wait_with_output().expect("sha256sum output");
    assert!(out.status.success(), "sha256sum failed");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum output")
        .to_string()
}

fn write_index(tmp: &TempDir, name: &str, version: &str, tarball_url: &str, sha256: &str) -> PathBuf {
    let toml = format!(
        "[versions.\"{version}\"]\n\
         tarball_url = \"{tarball_url}\"\n\
         sha256 = \"{sha256}\"\n"
    );
    tmp.write(&format!("index/packages/{name}.toml"), &toml);
    tmp.path().join("index")
}

fn install_opts(root: &Path) -> InstallOptions {
    InstallOptions {
        dest: Some(root.to_path_buf()),
        ..Default::default()
    }
}

fn root_opts(root: &Path) -> RootOptions {
    RootOptions {
        dest: Some(root.to_path_buf()),
        ..Default::default()
    }
}

/// A fresh, per-test archive-cache directory under this test's own `tmp`.
///
/// The archive cache (`crate::cache`) is content-addressed by sha256 ALONE
/// (`<cache_root>/<sha256>.tar.gz`, independent of package name/url), and when
/// `RegistryOptions::archive_cache_dir` is `None` it falls back to a single
/// machine-global root (`$XDG_CACHE_HOME/rustyfi/archives`, else
/// `$HOME/.cache/…`). Every `http(s)://` fetch goes through that cache, so two
/// installs of a byte-identical fixture tarball — as several of these tests
/// build (`great-package` / `"let great = 1\n"` ⇒ same bytes ⇒ same sha ⇒ same
/// cache file) — would SHARE the global entry. A warm entry written by a
/// concurrently-running (`--jobs 3` / multi-threaded test binary) or a
/// previously-run test then short-circuits a later test's fetch to ZERO GETs,
/// breaking its "server hit exactly once" / bearer-header assertions with a
/// spurious `left: 0` (no GET ever reached the loopback server). Pointing each
/// test at its OWN empty cache dir keeps the fetch genuinely cold and the test
/// hermetic, while still exercising the real `ureq`-over-loopback download.
fn isolated_cache(tmp: &TempDir) -> PathBuf {
    tmp.path().join("archive-cache")
}

fn assert_no_dist_or_receipts(root: &Path) {
    assert!(!root.join("dist").exists(), "dist/ must not be created");
    let receipts = root.join(".satyrographos/receipts");
    let count = fs::read_dir(&receipts)
        .map(|rd| rd.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    assert_eq!(count, 0, "no receipt may be written");
}

// A minimal loopback HTTP/1.1 server (no new dev-dependency): binds
// `127.0.0.1:0`, replies to `GET <path>` with a pre-registered `Route`. Each
// connection is handled on its own thread and closed after one response
// (`Connection: close`), so no keep-alive bookkeeping is needed.

enum Route {
    Body(Vec<u8>),
    Status(u16, &'static str),
    /// Accept the connection but never write a response — the client's own
    /// read timeout must fire.
    Stall,
}

struct MockServer {
    port: u16,
    hits: Arc<Mutex<Vec<String>>>,
    /// The `Authorization` header value seen on each request, in request
    /// order (`None` when the request carried none) — saphe 7d slice S3's
    /// bearer-token-auth coverage.
    auth_headers: Arc<Mutex<Vec<Option<String>>>>,
}

impl MockServer {
    fn start(routes: Vec<(&'static str, Route)>) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let port = listener.local_addr().unwrap().port();
        Self::spawn(listener, port, routes)
    }

    /// Like [`start`](Self::start), but `build_routes` receives the bound
    /// port *before* the route table is fixed — for a fixture whose route
    /// BODIES need to self-reference the server's own URL (e.g. a sparse
    /// index entry whose `tarball_url` points back at a tarball route on this
    /// same server). `start` cannot express this: its `routes` argument is
    /// built by the caller before the port is known.
    fn start_self_referencing(build_routes: impl FnOnce(u16) -> Vec<(&'static str, Route)>) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let port = listener.local_addr().unwrap().port();
        let routes = build_routes(port);
        Self::spawn(listener, port, routes)
    }

    fn spawn(listener: TcpListener, port: u16, routes: Vec<(&'static str, Route)>) -> MockServer {
        let routes: Arc<HashMap<String, Arc<Route>>> = Arc::new(
            routes
                .into_iter()
                .map(|(p, r)| (p.to_string(), Arc::new(r)))
                .collect(),
        );
        let hits = Arc::new(Mutex::new(Vec::new()));
        let hits_bg = hits.clone();
        let auth_headers = Arc::new(Mutex::new(Vec::new()));
        let auth_headers_bg = auth_headers.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let routes = routes.clone();
                let hits = hits_bg.clone();
                let auth_headers = auth_headers_bg.clone();
                thread::spawn(move || handle_conn(stream, &routes, &hits, &auth_headers));
            }
        });
        MockServer {
            port,
            hits,
            auth_headers,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    /// The server's bare origin (no path) — a mirror/sparse-index "base URL".
    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn hit_count(&self, path: &str) -> usize {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.as_str() == path)
            .count()
    }

    /// The `Authorization` header value of the most recent request, if any.
    fn last_authorization(&self) -> Option<String> {
        self.auth_headers.lock().unwrap().last().cloned().flatten()
    }
}

fn handle_conn(
    mut stream: TcpStream,
    routes: &HashMap<String, Arc<Route>>,
    hits: &Arc<Mutex<Vec<String>>>,
    auth_headers: &Arc<Mutex<Vec<Option<String>>>>,
) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    // Drain headers up to the blank line, capturing `Authorization` (the rest
    // we still don't need).
    let mut auth: Option<String> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if line == "\r\n" || line == "\n" {
                    break;
                }
                if let Some(rest) = line
                    .strip_prefix("Authorization:")
                    .or_else(|| line.strip_prefix("authorization:"))
                {
                    auth = Some(rest.trim().to_string());
                }
            }
        }
    }
    auth_headers.lock().unwrap().push(auth);
    hits.lock().unwrap().push(path.clone());

    match routes.get(&path) {
        Some(route) => match &**route {
            Route::Body(bytes) => {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(bytes);
            }
            Route::Status(code, reason) => {
                let header = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(header.as_bytes());
            }
            Route::Stall => {
                // Long enough to outlast the short client timeout the timeout
                // test configures, short enough not to leak an indefinitely
                // blocked thread.
                thread::sleep(Duration::from_secs(5));
            }
        },
        None => {
            let body = b"not found";
            let header = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
        }
    }
}

#[test]
fn http_registry_install_happy_path() {
    let tmp = TempDir::new("happy");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let server = MockServer::start(vec![("/great-package-1.0.0.tar.gz", Route::Body(bytes))]);
    let index = write_index(
        &tmp,
        "great-package",
        "1.0.0",
        &server.url("/great-package-1.0.0.tar.gz"),
        &sha,
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(format!("file://{}", index.display())),
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    let (report, resolved) =
        sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
            .expect("http registry install ok");

    assert_eq!(report.name, "great-package");
    assert_eq!(resolved.version, "1.0.0");
    assert_eq!(resolved.sha256, sha);
    assert!(resolved.url.starts_with("http://127.0.0.1"));

    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());

    let receipt = fs::read_to_string(root.join(".satyrographos/receipts/great-package.toml"))
        .expect("receipt written");
    assert!(receipt.contains("kind = \"registry\""), "{receipt}");
    assert!(receipt.contains("http://127.0.0.1"), "{receipt}");
    assert!(receipt.contains(&sha), "{receipt}");

    assert_eq!(server.hit_count("/great-package-1.0.0.tar.gz"), 1);
}

#[test]
fn http_registry_404_maps_to_http_failed_no_writes() {
    let tmp = TempDir::new("404");
    let server = MockServer::start(vec![]); // no routes registered -> every path 404s
    let index = write_index(
        &tmp,
        "great-package",
        "1.0.0",
        &server.url("/missing.tar.gz"),
        "0".repeat(64).as_str(),
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(format!("file://{}", index.display())),
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    let err = sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect_err("404 must fail");
    match &err {
        sg::Error::HttpFailed { url, message } => {
            assert!(url.contains("missing.tar.gz"), "{err}");
            assert!(message.contains("404"), "expected 404 in message: {message}");
        }
        other => panic!("expected HttpFailed, got {other}"),
    }
    assert_no_dist_or_receipts(&root);
}

#[test]
fn http_registry_500_maps_to_http_failed_no_writes() {
    let tmp = TempDir::new("500");
    let server = MockServer::start(vec![(
        "/broken.tar.gz",
        Route::Status(500, "Internal Server Error"),
    )]);
    let index = write_index(
        &tmp,
        "great-package",
        "1.0.0",
        &server.url("/broken.tar.gz"),
        "0".repeat(64).as_str(),
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(format!("file://{}", index.display())),
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    let err = sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect_err("500 must fail");
    match &err {
        sg::Error::HttpFailed { message, .. } => {
            assert!(message.contains("500"), "expected 500 in message: {message}");
        }
        other => panic!("expected HttpFailed, got {other}"),
    }
    assert_no_dist_or_receipts(&root);
}

// S1 checksum mismatch: server serves the real bytes honestly; the index
// lies about the sha256. Same "abort before dist/" guarantee as the file://
// transport (tests/registry.rs's sha256_mismatch_leaves_dist_and_receipts_untouched).

#[test]
fn http_registry_checksum_mismatch_no_dist_writes() {
    let tmp = TempDir::new("mismatch");
    let (bytes, real_sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let bad_sha = "0".repeat(64);
    assert_ne!(real_sha, bad_sha);
    let server = MockServer::start(vec![("/great-package-1.0.0.tar.gz", Route::Body(bytes))]);
    let index = write_index(
        &tmp,
        "great-package",
        "1.0.0",
        &server.url("/great-package-1.0.0.tar.gz"),
        &bad_sha,
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(format!("file://{}", index.display())),
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    let err = sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect_err("checksum mismatch must fail");
    assert!(
        matches!(err, sg::Error::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err}"
    );
    assert_no_dist_or_receipts(&root);

    assert_eq!(server.hit_count("/great-package-1.0.0.tar.gz"), 1);
    let tmp_dir = root.join(".satyrographos/tmp");
    let leftovers = fs::read_dir(&tmp_dir)
        .map(|rd| rd.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "unverified download must be cleaned up");
}

// S1 timeout: a stalled server must not hang the install past the configured
// budget. `RUSTYFI_HTTP_TIMEOUT` (`sg::registry::HTTP_TIMEOUT_ENV`) is set to a
// small value for the duration of this test so it does not have to wait out
// the real 30s default; `get_to_file` reads the env var fresh on every call,
// so this takes effect immediately and the process-wide mutation only needs
// to outlive this one synchronous call.

#[test]
fn http_registry_timeout_errors_within_configured_budget() {
    let tmp = TempDir::new("timeout");
    let server = MockServer::start(vec![("/stalls.tar.gz", Route::Stall)]);
    let index = write_index(
        &tmp,
        "great-package",
        "1.0.0",
        &server.url("/stalls.tar.gz"),
        "0".repeat(64).as_str(),
    );
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(format!("file://{}", index.display())),
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    // SAFETY: std::env::set_var/remove_var are `unsafe` since they are not
    // thread-safe against concurrent env reads in other threads; this test is
    // the only place in the suite that touches RUSTYFI_HTTP_TIMEOUT, and every
    // other test's http call completes in milliseconds against the loopback
    // server regardless of the timeout budget, so a race would not make them
    // flaky even if one occurred.
    unsafe {
        std::env::set_var(sg::registry::HTTP_TIMEOUT_ENV, "1");
    }
    let started = Instant::now();
    let err = sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect_err("a stalled server must time out, not hang");
    let elapsed = started.elapsed();
    unsafe {
        std::env::remove_var(sg::registry::HTTP_TIMEOUT_ENV);
    }

    assert!(
        matches!(err, sg::Error::HttpFailed { .. }),
        "expected HttpFailed (timeout), got {err}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "timeout took too long: {elapsed:?} (configured budget was 1s; \
         the real default is 30s, so this proves the override took effect)"
    );
    assert_no_dist_or_receipts(&root);
}

// Saphe 7d slice S3: bearer-token auth. `RUSTYFI_REGISTRY_TOKEN`
// (`sg::registry::REGISTRY_TOKEN_ENV`) is attached as `Authorization:
// Bearer <token>` on every tarball GET; `get_to_file` reads it fresh per call,
// same discipline as the timeout test's `RUSTYFI_HTTP_TIMEOUT` above.

// Both the "no token configured" and "token configured" cases live in ONE
// test function (rather than two `#[test]`s) so the `RUSTYFI_REGISTRY_TOKEN`
// mutation can never race a concurrently-running sibling test that asserts on
// the header's *absence* — unlike the timeout test's env var (whose value
// does not change another test's pass/fail outcome), a stray token would flip
// this crate's own no-auth assertion, so it must be fully sequential.
#[test]
fn http_registry_bearer_token_header_matches_configuration() {
    let tmp = TempDir::new("auth");

    {
        let (bytes, sha) = make_tarball_bytes(&tmp, "no-token-pkg", "1.0.0", "let x = 1\n");
        let server = MockServer::start(vec![("/no-token-pkg-1.0.0.tar.gz", Route::Body(bytes))]);
        let index = write_index(
            &tmp,
            "no-token-pkg",
            "1.0.0",
            &server.url("/no-token-pkg-1.0.0.tar.gz"),
            &sha,
        );
        let root = tmp.path().join("root-no-token");
        let reg = RegistryOptions {
            url: Some(format!("file://{}", index.display())),
            archive_cache_dir: Some(tmp.path().join("archive-cache-no-token")),
            ..Default::default()
        };
        sg::install_registry("no-token-pkg", None, &install_opts(&root), &reg, None)
            .expect("install ok");
        assert_eq!(
            server.last_authorization(),
            None,
            "no Authorization header without RUSTYFI_REGISTRY_TOKEN set"
        );
    }

    {
        let (bytes, sha) = make_tarball_bytes(&tmp, "token-pkg", "1.0.0", "let y = 1\n");
        let server = MockServer::start(vec![("/token-pkg-1.0.0.tar.gz", Route::Body(bytes))]);
        let index = write_index(
            &tmp,
            "token-pkg",
            "1.0.0",
            &server.url("/token-pkg-1.0.0.tar.gz"),
            &sha,
        );
        let root = tmp.path().join("root-token");
        let reg = RegistryOptions {
            url: Some(format!("file://{}", index.display())),
            archive_cache_dir: Some(tmp.path().join("archive-cache-token")),
            ..Default::default()
        };

        // SAFETY: std::env::set_var/remove_var are `unsafe` since they are
        // not thread-safe against concurrent env reads in other threads;
        // this is the only test in the suite that touches
        // RUSTYFI_REGISTRY_TOKEN, and both the mutation and the one call it
        // affects happen back-to-back within this single test function
        // (never interleaved with the "no token" case above, which already
        // completed).
        unsafe {
            std::env::set_var(sg::registry::REGISTRY_TOKEN_ENV, "s3kr1t");
        }
        let result = sg::install_registry("token-pkg", None, &install_opts(&root), &reg, None);
        unsafe {
            std::env::remove_var(sg::registry::REGISTRY_TOKEN_ENV);
        }
        result.expect("install with a configured token still succeeds");

        assert_eq!(
            server.last_authorization(),
            Some("Bearer s3kr1t".to_string()),
            "the tarball GET must carry the configured bearer token"
        );
    }
}

// Saphe phase 7d slice S2: content-addressed archive cache + `--offline`.
// Every case here shares ONE mock server (a real `ureq`
// GET, request-counted) and ONE archive-cache dir across MULTIPLE installs
// into SEPARATE fresh roots — so a second install that hits ZERO GETs proves
// the *cache* short-circuited the fetch, not the reconcile receipt-skip
// (which a same-root re-install would trigger for an unrelated reason).

/// The index is always local (never network), so these tests isolate the
/// *archive* cache / offline behaviour, not index acquisition.
fn reg_cached(index: &Path, cache: &Path, offline: bool) -> RegistryOptions {
    RegistryOptions {
        url: Some(format!("file://{}", index.display())),
        archive_cache_dir: Some(cache.to_path_buf()),
        offline,
        ..Default::default()
    }
}

/// S2 — warm cache ⇒ zero GETs. First install (cold cache) fetches once;
/// a second install into a *fresh* root sharing the same archive cache issues
/// ZERO additional archive GETs and still materialises the package.
#[test]
fn s2_warm_cache_second_install_makes_zero_gets() {
    let tmp = TempDir::new("s2-warm");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let server = MockServer::start(vec![("/great-package-1.0.0.tar.gz", Route::Body(bytes))]);
    let route = "/great-package-1.0.0.tar.gz";
    let index = write_index(&tmp, "great-package", "1.0.0", &server.url(route), &sha);
    let cache = tmp.path().join("archive-cache");

    let root1 = tmp.path().join("root1");
    sg::install_registry(
        "great-package",
        None,
        &install_opts(&root1),
        &reg_cached(&index, &cache, false),
        None,
    )
    .expect("cold install ok");
    assert_eq!(server.hit_count(route), 1, "cold install fetches once");
    assert!(cache.join(format!("{sha}.tar.gz")).is_file(), "cache populated by sha");

    let root2 = tmp.path().join("root2");
    sg::install_registry(
        "great-package",
        None,
        &install_opts(&root2),
        &reg_cached(&index, &cache, false),
        None,
    )
    .expect("warm install ok");
    assert_eq!(server.hit_count(route), 1, "warm install must issue ZERO new GETs");
    assert!(root2
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
}

/// S2 — `--offline` with a warm cache succeeds with zero GETs.
#[test]
fn s2_offline_warm_cache_succeeds_zero_gets() {
    let tmp = TempDir::new("s2-offline-warm");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let server = MockServer::start(vec![("/great-package-1.0.0.tar.gz", Route::Body(bytes))]);
    let route = "/great-package-1.0.0.tar.gz";
    let index = write_index(&tmp, "great-package", "1.0.0", &server.url(route), &sha);
    let cache = tmp.path().join("archive-cache");

    sg::install_registry(
        "great-package",
        None,
        &install_opts(&tmp.path().join("warm")),
        &reg_cached(&index, &cache, false),
        None,
    )
    .expect("warm-up install ok");
    assert_eq!(server.hit_count(route), 1);

    let root = tmp.path().join("offline-root");
    sg::install_registry(
        "great-package",
        None,
        &install_opts(&root),
        &reg_cached(&index, &cache, true),
        None,
    )
    .expect("offline install from warm cache ok");
    assert_eq!(server.hit_count(route), 1, "offline warm install must issue ZERO GETs");
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
}

/// S2 — `--offline` with a COLD cache errors cleanly (`Error::Offline`) and
/// makes no network attempt at all.
#[test]
fn s2_offline_cold_cache_errors_without_network() {
    let tmp = TempDir::new("s2-offline-cold");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let server = MockServer::start(vec![("/great-package-1.0.0.tar.gz", Route::Body(bytes))]);
    let route = "/great-package-1.0.0.tar.gz";
    let index = write_index(&tmp, "great-package", "1.0.0", &server.url(route), &sha);
    let cache = tmp.path().join("empty-cache");

    let root = tmp.path().join("root");
    let err = sg::install_registry(
        "great-package",
        None,
        &install_opts(&root),
        &reg_cached(&index, &cache, true),
        None,
    )
    .expect_err("offline + cold cache must error, not fetch");
    match &err {
        sg::Error::Offline { url } => assert!(url.contains("great-package-1.0.0.tar.gz"), "{err}"),
        other => panic!("expected Error::Offline, got {other}"),
    }
    assert_eq!(server.hit_count(route), 0, "offline must not touch the network");
    assert_no_dist_or_receipts(&root);
}

/// S2 — a cached archive whose bytes got corrupted on disk is NOT trusted: an
/// online install re-fetches (the corrupt entry re-verifies against neither
/// sha and is discarded), repairs the cache, and installs; an offline install
/// against the same corruption cannot re-fetch and errors cleanly.
#[test]
fn s2_corrupted_cache_entry_is_refetched_and_offline_errors() {
    let tmp = TempDir::new("s2-corrupt");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let server = MockServer::start(vec![("/great-package-1.0.0.tar.gz", Route::Body(bytes))]);
    let route = "/great-package-1.0.0.tar.gz";
    let index = write_index(&tmp, "great-package", "1.0.0", &server.url(route), &sha);
    let cache = tmp.path().join("archive-cache");

    sg::install_registry(
        "great-package",
        None,
        &install_opts(&tmp.path().join("warm")),
        &reg_cached(&index, &cache, false),
        None,
    )
    .expect("warm-up install ok");
    assert_eq!(server.hit_count(route), 1);
    let cached_path = cache.join(format!("{sha}.tar.gz"));
    assert!(cached_path.is_file());
    fs::write(&cached_path, b"corrupted not-a-tarball").expect("corrupt the cache entry");

    let offline_root = tmp.path().join("offline-root");
    let err = sg::install_registry(
        "great-package",
        None,
        &install_opts(&offline_root),
        &reg_cached(&index, &cache, true),
        None,
    )
    .expect_err("corrupt cache + offline must error, not install bad bytes");
    assert!(
        matches!(err, sg::Error::Offline { .. }),
        "expected Error::Offline, got {err}"
    );
    assert_eq!(server.hit_count(route), 1, "offline must not re-fetch");
    assert_no_dist_or_receipts(&offline_root);

    let online_root = tmp.path().join("online-root");
    sg::install_registry(
        "great-package",
        None,
        &install_opts(&online_root),
        &reg_cached(&index, &cache, false),
        None,
    )
    .expect("corrupt cache online must re-fetch and install");
    assert_eq!(server.hit_count(route), 2, "corrupt entry must trigger exactly one re-fetch");
    assert!(online_root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
    assert_eq!(
        sha256_of_bytes(&fs::read(&cached_path).unwrap()),
        sha,
        "cache entry repaired to honest bytes"
    );
}

// Slice M: registry mirror-list fallback. Two independent loopback servers
// stand in for a primary registry host and a mirror host;
// `RegistryOptions::mirrors` is a **bare origin** (a mirror is a host/prefix
// substitution applied to the primary URL's own path — [`MockServer::
// base_url`]), rewritten against the index's `tarball_url` by
// `registry::rewrite_to_mirror`.

fn reg_with_mirrors(index: &Path, cache: &Path, mirrors: Vec<String>) -> RegistryOptions {
    RegistryOptions {
        url: Some(format!("file://{}", index.display())),
        archive_cache_dir: Some(cache.to_path_buf()),
        mirrors,
        ..Default::default()
    }
}

/// M — primary 500s, the mirror serves the real bytes: install succeeds via
/// the mirror after exactly one failed primary attempt.
#[test]
fn mirrors_primary_500_falls_back_to_mirror_200() {
    let tmp = TempDir::new("mirrors-fallback");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let path = "/great-package-1.0.0.tar.gz";
    let primary = MockServer::start(vec![(path, Route::Status(500, "Internal Server Error"))]);
    let mirror = MockServer::start(vec![(path, Route::Body(bytes))]);
    let index = write_index(&tmp, "great-package", "1.0.0", &primary.url(path), &sha);
    let root = tmp.path().join("root");
    let reg = reg_with_mirrors(&index, &isolated_cache(&tmp), vec![mirror.base_url()]);

    let (report, resolved) =
        sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
            .expect("install must succeed via the mirror");
    assert_eq!(report.name, "great-package");
    assert_eq!(resolved.sha256, sha);
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());

    assert_eq!(primary.hit_count(path), 1, "exactly one failed primary attempt");
    assert_eq!(mirror.hit_count(path), 1, "the mirror serves the successful fetch");
}

/// M — the primary serves the WRONG bytes (a checksum mismatch, not an HTTP
/// error); the mirror serves the correct bytes. Verification happens
/// per-candidate BEFORE any cache write, so the bad bytes never poison the
/// cache — the next candidate is tried and the cache ends up holding only the
/// honest bytes.
#[test]
fn mirrors_bad_bytes_falls_through_to_good_mirror() {
    let tmp = TempDir::new("mirrors-bad-bytes");
    let (good_bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let bad_bytes = b"not the right tarball bytes at all".to_vec();
    let path = "/great-package-1.0.0.tar.gz";
    let primary = MockServer::start(vec![(path, Route::Body(bad_bytes))]);
    let mirror = MockServer::start(vec![(path, Route::Body(good_bytes))]);
    let index = write_index(&tmp, "great-package", "1.0.0", &primary.url(path), &sha);
    let root = tmp.path().join("root");
    let cache = isolated_cache(&tmp);
    let reg = reg_with_mirrors(&index, &cache, vec![mirror.base_url()]);

    sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect("install must fall through the bad-bytes primary to the good mirror");

    assert_eq!(primary.hit_count(path), 1, "the corrupt primary is tried once");
    assert_eq!(mirror.hit_count(path), 1, "the good mirror serves the successful fetch");
    let cached_path = cache.join(format!("{sha}.tar.gz"));
    assert_eq!(
        sha256_of_bytes(&fs::read(&cached_path).unwrap()),
        sha,
        "cache holds the verified (mirror) bytes, never the corrupt primary bytes"
    );
}

/// M — every candidate (primary + every mirror) fails: the install errors and
/// touches neither `dist/` nor the archive cache.
#[test]
fn mirrors_all_fail_returns_error() {
    let tmp = TempDir::new("mirrors-all-fail");
    let path = "/great-package-1.0.0.tar.gz";
    let primary = MockServer::start(vec![(path, Route::Status(500, "Internal Server Error"))]);
    let mirror = MockServer::start(vec![(path, Route::Status(503, "Service Unavailable"))]);
    let index = write_index(&tmp, "great-package", "1.0.0", &primary.url(path), &"0".repeat(64));
    let root = tmp.path().join("root");
    let cache = isolated_cache(&tmp);
    let reg = reg_with_mirrors(&index, &cache, vec![mirror.base_url()]);

    let err = sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect_err("every candidate failing must error");
    match &err {
        sg::Error::HttpFailed { message, .. } => {
            assert!(message.contains("503"), "expected the LAST candidate's error (503): {message}");
        }
        other => panic!("expected HttpFailed, got {other}"),
    }
    assert_eq!(primary.hit_count(path), 1);
    assert_eq!(mirror.hit_count(path), 1);
    assert_no_dist_or_receipts(&root);
    assert!(!cache.join(format!("{}.tar.gz", "0".repeat(64))).exists());
}

/// M — a warm cache short-circuits BEFORE any candidate URL is tried (the
/// cache key is the sha256, mirror-independent): a second install sharing the
/// same archive cache issues ZERO new GETs to either the primary or any
/// configured mirror, even though mirrors are present in `RegistryOptions`.
#[test]
fn mirrors_warm_cache_tries_zero_urls() {
    let tmp = TempDir::new("mirrors-warm-cache");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let path = "/great-package-1.0.0.tar.gz";
    let primary = MockServer::start(vec![(path, Route::Body(bytes))]);
    // The mirror would fail if ever hit — proving it genuinely never is.
    let mirror = MockServer::start(vec![(path, Route::Status(500, "Internal Server Error"))]);
    let index = write_index(&tmp, "great-package", "1.0.0", &primary.url(path), &sha);
    let cache = isolated_cache(&tmp);

    let root1 = tmp.path().join("root1");
    let reg_cold = reg_with_mirrors(&index, &cache, vec![]);
    sg::install_registry("great-package", None, &install_opts(&root1), &reg_cold, None)
        .expect("cold install ok");
    assert_eq!(primary.hit_count(path), 1, "cold install fetches once");

    let root2 = tmp.path().join("root2");
    let reg_warm = reg_with_mirrors(&index, &cache, vec![mirror.base_url()]);
    sg::install_registry("great-package", None, &install_opts(&root2), &reg_warm, None)
        .expect("warm install ok");
    assert_eq!(primary.hit_count(path), 1, "warm install must issue ZERO new primary GETs");
    assert_eq!(mirror.hit_count(path), 0, "warm install must never touch a configured mirror");
    assert!(root2
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
}

// Slice S: sparse HTTP index. `packages/<name>.toml` is fetched
// on demand over HTTP from a mocked loopback server instead of being
// git-cloned/read from a plain directory — `RegistryConfig`/`RegistryOptions`
// `kind = Sparse` selects the transport; `lookup` is backend-aware; the
// solver (`solve.rs`/`RegistryDepSource`) is untouched.

/// A `[versions."<v>"]` sparse-index TOML body, matching the exact
/// `PackageIndex`/`VersionEntry` schema a local/git index already parses (no
/// new file format) — optionally declaring `deps`.
fn sparse_index_toml(version: &str, tarball_url: &str, sha256: &str, deps: &[(&str, &str)]) -> Vec<u8> {
    let mut toml = format!(
        "[versions.\"{version}\"]\n\
         tarball_url = \"{tarball_url}\"\n\
         sha256 = \"{sha256}\"\n"
    );
    if !deps.is_empty() {
        toml.push_str(&format!("\n[versions.\"{version}\".dependencies]\n"));
        for (dep, c) in deps {
            toml.push_str(&format!("{dep} = \"{c}\"\n"));
        }
    }
    toml.into_bytes()
}

/// A `Satyristes` declaring a sparse `(registry ...)` (`url`/`kind`, plus
/// `mirrors` when non-empty) and one registry-sourced dependency.
fn write_sparse_manifest(tmp: &TempDir, base_url: &str, mirrors: &[String], pkg: &str, version: &str) -> PathBuf {
    let mirrors_form = if mirrors.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = mirrors.iter().map(|m| format!("\"{m}\"")).collect();
        format!(" (mirrors ({}))", quoted.join(" "))
    };
    tmp.write(
        "proj/Satyristes",
        &format!(
            "(version 0.0.2)\n\
             (registry (url \"{base_url}\") (kind sparse){mirrors_form})\n\
             (library (name \"proj\") (version \"0.1.0\")\n\
               (sources ((packageDir \"src\")))\n\
               (dependencies (({pkg} ((registry \"{pkg}\") (version \"{version}\"))))))\n"
        ),
    )
}

/// S — the solver only GETs `packages/<name>.toml` for packages it actually
/// visits: `great-package` depends on `base-package`; a third, unrelated
/// package sits in the index but is never named by anything and must get
/// ZERO GETs.
#[test]
fn sparse_solve_fetches_per_package() {
    let tmp = TempDir::new("sparse-solve");
    let (great_bytes, great_sha) =
        make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let (base_bytes, base_sha) = make_tarball_bytes(&tmp, "base-package", "1.0.0", "let base = 1\n");
    let (unrelated_bytes, unrelated_sha) =
        make_tarball_bytes(&tmp, "unrelated", "1.0.0", "let unrelated = 1\n");

    // The index entries' `tarball_url`s self-reference this same server, so
    // the route bodies are built from the (otherwise unknown until bind)
    // port via `start_self_referencing`.
    let server = MockServer::start_self_referencing(|port| {
        let base = format!("http://127.0.0.1:{port}");
        vec![
            (
                "/packages/great-package.toml",
                Route::Body(sparse_index_toml(
                    "1.0.0",
                    &format!("{base}/tarballs/great-package-1.0.0.tar.gz"),
                    &great_sha,
                    &[("base-package", "^1.0.0")],
                )),
            ),
            (
                "/packages/base-package.toml",
                Route::Body(sparse_index_toml(
                    "1.0.0",
                    &format!("{base}/tarballs/base-package-1.0.0.tar.gz"),
                    &base_sha,
                    &[],
                )),
            ),
            (
                "/packages/unrelated.toml",
                Route::Body(sparse_index_toml(
                    "1.0.0",
                    &format!("{base}/tarballs/unrelated-1.0.0.tar.gz"),
                    &unrelated_sha,
                    &[],
                )),
            ),
            ("/tarballs/great-package-1.0.0.tar.gz", Route::Body(great_bytes)),
            ("/tarballs/base-package-1.0.0.tar.gz", Route::Body(base_bytes)),
            ("/tarballs/unrelated-1.0.0.tar.gz", Route::Body(unrelated_bytes)),
        ]
    });

    let manifest = write_sparse_manifest(&tmp, &server.base_url(), &[], "great-package", "1.0.0");
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    let report = sg::install_manifest_reg(&manifest, &root_opts(&root), &reg)
        .expect("sparse reconcile resolves the closure");
    assert_eq!(report.installed.len(), 2, "great-package + its transitive base-package");
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
    assert!(root
        .join("dist/packages/base-package/base-package.satyh")
        .is_file());

    assert!(server.hit_count("/packages/great-package.toml") >= 1, "the visited direct root is GET'd");
    assert!(
        server.hit_count("/packages/base-package.toml") >= 1,
        "the visited transitive dependency is GET'd"
    );
    assert_eq!(
        server.hit_count("/packages/unrelated.toml"),
        0,
        "an unvisited package in the index must get ZERO GETs"
    );
}

/// S — `--offline` with nothing yet fetched in this run errors cleanly
/// ([`sg::Error::Offline`]) without ever touching the network, for a Sparse
/// registry exactly as it already does for a git/local one.
#[test]
fn sparse_offline_uncached_errors() {
    let tmp = TempDir::new("sparse-offline");
    let server = MockServer::start(vec![(
        "/packages/great-package.toml",
        Route::Body(sparse_index_toml("1.0.0", "http://unused/", &"0".repeat(64), &[])),
    )]);
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(server.base_url()),
        kind: Some(RegistryKind::Sparse),
        offline: true,
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    let err = sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
        .expect_err("offline + nothing cached in this run must error, not fetch");
    assert!(matches!(err, sg::Error::Offline { .. }), "expected Error::Offline, got {err}");
    assert_eq!(
        server.hit_count("/packages/great-package.toml"),
        0,
        "offline must never touch the sparse index endpoint"
    );
}

/// S composes with M: the primary sparse base 500s on the per-package GET;
/// the mirror sparse base serves the real index file (and the tarball),
/// reusing Slice M's candidate-list fallback for the sparse index fetch.
#[test]
fn sparse_with_mirrors_falls_back() {
    let tmp = TempDir::new("sparse-mirrors");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let primary = MockServer::start(vec![(
        "/packages/great-package.toml",
        Route::Status(500, "Internal Server Error"),
    )]);
    let mirror = MockServer::start_self_referencing(|port| {
        let base = format!("http://127.0.0.1:{port}");
        vec![
            (
                "/packages/great-package.toml",
                Route::Body(sparse_index_toml(
                    "1.0.0",
                    &format!("{base}/tarballs/great-package-1.0.0.tar.gz"),
                    &sha,
                    &[],
                )),
            ),
            ("/tarballs/great-package-1.0.0.tar.gz", Route::Body(bytes)),
        ]
    });

    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        url: Some(primary.base_url()),
        kind: Some(RegistryKind::Sparse),
        mirrors: vec![mirror.base_url()],
        archive_cache_dir: Some(isolated_cache(&tmp)),
        ..Default::default()
    };

    let (report, resolved) =
        sg::install_registry("great-package", None, &install_opts(&root), &reg, None)
            .expect("sparse index lookup must fall back to the mirror");
    assert_eq!(report.name, "great-package");
    assert_eq!(resolved.sha256, sha);
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
    assert_eq!(primary.hit_count("/packages/great-package.toml"), 1);
}

/// S — a lockfile fully satisfying the roots skips the index entirely (the
/// `all_reusable` reconcile fast path): after an initial online sparse
/// reconcile, a second install against a fresh destination root but
/// the SAME lockfile, offline, succeeds via the warm archive cache with ZERO
/// further GETs to the sparse index endpoint.
#[test]
fn sparse_lockfile_reproducible_offline() {
    let tmp = TempDir::new("sparse-lock-offline");
    let (bytes, sha) = make_tarball_bytes(&tmp, "great-package", "1.0.0", "let great = 1\n");
    let server = MockServer::start_self_referencing(|port| {
        let base = format!("http://127.0.0.1:{port}");
        vec![
            (
                "/packages/great-package.toml",
                Route::Body(sparse_index_toml(
                    "1.0.0",
                    &format!("{base}/tarballs/great-package-1.0.0.tar.gz"),
                    &sha,
                    &[],
                )),
            ),
            ("/tarballs/great-package-1.0.0.tar.gz", Route::Body(bytes)),
        ]
    });

    let manifest = write_sparse_manifest(&tmp, &server.base_url(), &[], "great-package", "1.0.0");
    let cache = isolated_cache(&tmp);

    let root1 = tmp.path().join("root1");
    let reg_online = RegistryOptions {
        archive_cache_dir: Some(cache.clone()),
        ..Default::default()
    };
    sg::install_manifest_reg(&manifest, &root_opts(&root1), &reg_online).expect("first reconcile ok");
    assert_eq!(server.hit_count("/packages/great-package.toml"), 1, "first reconcile consults the index once");
    assert_eq!(server.hit_count("/tarballs/great-package-1.0.0.tar.gz"), 1);

    // Second: fresh destination root, offline, same manifest/lock. The
    // `all_reusable` fast path (reconcile.rs) must skip the index entirely
    // (no `registry::acquire`/`lookup` at all — sparse or otherwise) and
    // rematerialise purely from the warm archive cache.
    let root2 = tmp.path().join("root2");
    let reg_offline = RegistryOptions {
        archive_cache_dir: Some(cache),
        offline: true,
        ..Default::default()
    };
    sg::install_manifest_reg(&manifest, &root_opts(&root2), &reg_offline)
        .expect("offline reconcile from a complete lockfile must succeed with no index access");
    assert!(root2
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());
    assert_eq!(
        server.hit_count("/packages/great-package.toml"),
        1,
        "offline reconcile must NOT touch the sparse index endpoint again"
    );
    assert_eq!(
        server.hit_count("/tarballs/great-package-1.0.0.tar.gz"),
        1,
        "offline reconcile must NOT re-fetch the tarball (warm archive cache)"
    );
}
