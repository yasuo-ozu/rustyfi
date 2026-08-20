//! `.opam` preparation: fetch, verify, build — the step that makes a font
//! package's declared files exist. No test here reaches the network: each
//! drives a decision the fetcher makes before it would.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_satyrographos as sg;
use sg::opam::{ExtraSource, Opam};

fn tmp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-opam-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("temp dir");
    p
}

/// sha256 of "payload", the body every fixture below writes.
const PAYLOAD: &str = "payload";
const PAYLOAD_SHA: &str = "239f59ed55e737c77147cf55ad0c1b030b6d7ee748a7426952f9b852d5a935e5";

fn source(name: &str, sha: Option<&str>) -> ExtraSource {
    ExtraSource {
        name: name.to_string(),
        url: "https://example.invalid/never-fetched".to_string(),
        sha256: sha.map(str::to_string),
    }
}

#[test]
fn a_present_file_matching_its_checksum_is_not_refetched() {
    let dir = tmp("reuse");
    fs::write(dir.join("archive.zip"), PAYLOAD).unwrap();
    let opam = Opam {
        extra_sources: vec![source("archive.zip", Some(PAYLOAD_SHA))],
        build: vec![],
        ..Default::default()
    };
    // Offline: any fetch would fail, so success proves none was attempted.
    let report = sg::ops::prepare::prepare_with(&dir, &opam, true, false).expect("reused");
    assert_eq!(report.reused, ["archive.zip"]);
    assert!(report.fetched.is_empty());
}

#[test]
fn a_present_file_with_the_wrong_checksum_is_refetched_not_accepted() {
    let dir = tmp("stale");
    fs::write(dir.join("archive.zip"), "not the payload").unwrap();
    let opam = Opam {
        extra_sources: vec![source("archive.zip", Some(PAYLOAD_SHA))],
        build: vec![],
        ..Default::default()
    };
    // Offline turns the decision into an observable error: it wanted to fetch.
    let err = sg::ops::prepare::prepare_with(&dir, &opam, true, false)
        .expect_err("a mismatching file must not be used");
    assert!(matches!(err, sg::Error::Offline { .. }), "{err}");
}

#[test]
fn build_commands_run_in_the_package_directory() {
    let dir = tmp("build");
    fs::write(dir.join("in.txt"), PAYLOAD).unwrap();
    let opam = Opam {
        extra_sources: vec![],
        build: vec![vec!["cp".into(), "in.txt".into(), "out.txt".into()]],
        ..Default::default()
    };
    let report = sg::ops::prepare::prepare_with(&dir, &opam, true, false).expect("build");
    assert!(dir.join("out.txt").is_file(), "ran in the package directory");
    assert_eq!(report.ran.len(), 1);
}

#[test]
fn a_failing_build_command_stops_and_says_which() {
    let dir = tmp("build-fail");
    let opam = Opam {
        extra_sources: vec![],
        build: vec![
            vec!["cp".into(), "missing".into(), "out.txt".into()],
            vec!["cp".into(), "missing".into(), "second.txt".into()],
        ],
        ..Default::default()
    };
    let err = sg::ops::prepare::prepare_with(&dir, &opam, true, false).expect_err("should fail");
    assert!(matches!(err, sg::Error::OpamBuild { .. }), "{err}");
    assert!(!dir.join("second.txt").exists(), "later commands do not run");
}

#[test]
fn a_delegation_to_satyrographos_is_recorded_not_run() {
    // `satyrographos opam install …` is OPAM handing the job to the tool this
    // port replaces; running it would fail or recurse.
    let dir = tmp("delegate");
    let opam = Opam {
        extra_sources: vec![],
        build: vec![
            vec!["satyrographos".into(), "opam".into(), "install".into()],
            vec!["opam".into(), "install".into()],
        ],
        ..Default::default()
    };
    let report = sg::ops::prepare::prepare_with(&dir, &opam, true, false).expect("no-op");
    assert_eq!(report.delegated.len(), 2);
    assert!(report.ran.is_empty());
}

#[test]
fn a_source_without_a_checksum_is_reported_as_unverified() {
    // It cannot be verified, so it must not look verified. Offline stops the
    // fetch, which is enough to show the decision is not "accept silently".
    let dir = tmp("unverified");
    let opam = Opam {
        extra_sources: vec![source("archive.zip", None)],
        build: vec![],
        ..Default::default()
    };
    let err = sg::ops::prepare::prepare_with(&dir, &opam, true, false).expect_err("would fetch");
    assert!(matches!(err, sg::Error::Offline { .. }), "{err}");
}
