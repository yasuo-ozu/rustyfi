//! `dist/hash/*.satysfi-hash` is CO-OWNED: every font package contributes
//! entries to the one file, so installing merges into it BY KEY (never
//! overwrites) and uninstalling takes only that package's keys back out.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_satyrographos as sg;
use sg::hashfile::HashFile;

fn tmp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-hash-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("temp dir");
    p
}

/// A root holding a standard library's own `fonts.satysfi-hash` — the state
/// every real root is in.
fn root_with_stdlib(dir: &Path) -> PathBuf {
    let root = dir.join("root");
    fs::create_dir_all(root.join("dist/hash")).unwrap();
    fs::create_dir_all(root.join("dist/fonts")).unwrap();
    fs::write(
        root.join("dist/hash/fonts.satysfi-hash"),
        r#"{ "ipaexm": { "src": "dist/fonts/ipaexm.ttf" } }"#,
    )
    .unwrap();
    root
}

/// A font package shipping one face and the hash entry that names it, in
/// upstream's spelling (Yojson variant syntax).
fn font_package(dir: &Path, name: &str, abbrev: &str) -> PathBuf {
    let src = dir.join(name);
    fs::create_dir_all(src.join("faces")).unwrap();
    fs::write(src.join(format!("faces/{abbrev}.otf")), "not really a font").unwrap();
    fs::write(
        src.join("fonts.satysfi-hash"),
        format!(r#"{{ "{abbrev}": <Single: {{ "src-dist": "{name}/{abbrev}.otf" }}> }}"#),
    )
    .unwrap();
    fs::write(
        src.join("Satyristes"),
        format!(
            r#"(version "0.0.3")
(library
  (name "{name}")
  (version "1.0")
  (sources ((fontDir "faces") (hash "fonts.satysfi-hash" "fonts.satysfi-hash"))))
"#
        ),
    )
    .unwrap();
    src
}

// The error type is the crate's own, which is large; a test that asserts on
// which variant came back has to name it.
#[allow(clippy::result_large_err)]
fn install(src: &Path, root: &Path, force: bool) -> Result<sg::ops::install::InstallReport, sg::Error> {
    sg::ops::install::install(
        src,
        &sg::ops::install::InstallOptions {
            dest: Some(root.to_path_buf()),
            force,
            ..Default::default()
        },
    )
}

fn live(root: &Path) -> HashFile {
    let text = fs::read_to_string(root.join("dist/hash/fonts.satysfi-hash")).unwrap();
    HashFile::parse(&text).expect("the file stays a readable hash file")
}

fn keys(root: &Path) -> Vec<String> {
    live(root).keys().map(str::to_string).collect()
}

#[test]
fn installing_a_font_package_keeps_the_standard_librarys_entries() {
    let dir = tmp("merge");
    let root = root_with_stdlib(&dir);
    let pkg = font_package(&dir, "fonts-a", "FaceA");

    install(&pkg, &root, false).expect("a pre-existing shared hash file is not a collision");

    assert_eq!(keys(&root), ["ipaexm", "FaceA"]);
}

#[test]
fn two_font_packages_coexist_in_one_file() {
    let dir = tmp("two");
    let root = root_with_stdlib(&dir);
    install(&font_package(&dir, "fonts-a", "FaceA"), &root, false).unwrap();
    install(&font_package(&dir, "fonts-b", "FaceB"), &root, false).unwrap();

    assert_eq!(keys(&root), ["ipaexm", "FaceA", "FaceB"]);
}

#[test]
fn a_packages_own_entry_survives_its_yojson_spelling() {
    let dir = tmp("yojson");
    let root = root_with_stdlib(&dir);
    install(&font_package(&dir, "fonts-a", "FaceA"), &root, false).unwrap();

    let text = fs::read_to_string(root.join("dist/hash/fonts.satysfi-hash")).unwrap();
    assert!(
        text.contains(r#"<Single: { "src-dist": "fonts-a/FaceA.otf" }>"#),
        "the package author's own declaration must not be rewritten: {text}"
    );
}

#[test]
fn uninstalling_takes_only_its_own_keys_out() {
    let dir = tmp("uninstall");
    let root = root_with_stdlib(&dir);
    install(&font_package(&dir, "fonts-a", "FaceA"), &root, false).unwrap();
    install(&font_package(&dir, "fonts-b", "FaceB"), &root, false).unwrap();

    sg::ops::uninstall::uninstall(
        "fonts-a",
        &sg::ops::uninstall::RootOptions {
            dest: Some(root.clone()),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(
        keys(&root),
        ["ipaexm", "FaceB"],
        "the other package's font and the standard library's must both remain"
    );
    assert!(
        !root.join("dist/fonts/fonts-a").exists(),
        "its own files still go"
    );
}

#[test]
fn a_force_reinstall_does_not_conflict_with_itself() {
    let dir = tmp("force");
    let root = root_with_stdlib(&dir);
    let pkg = font_package(&dir, "fonts-a", "FaceA");
    install(&pkg, &root, false).unwrap();

    install(&pkg, &root, true).expect("reinstalling replaces the package's own keys");

    assert_eq!(
        keys(&root),
        ["ipaexm", "FaceA"],
        "and does not duplicate them"
    );
}

#[test]
fn two_packages_claiming_one_font_name_is_refused() {
    let dir = tmp("clash");
    let root = root_with_stdlib(&dir);
    install(&font_package(&dir, "fonts-a", "FaceA"), &root, false).unwrap();
    let rival = font_package(&dir, "fonts-b", "FaceA");

    let err = install(&rival, &root, false).expect_err("silently picking a winner is not allowed");
    assert!(
        matches!(err, sg::Error::HashKeyConflict { .. }),
        "expected a key conflict, got {err}"
    );
    assert_eq!(
        keys(&root),
        ["ipaexm", "FaceA"],
        "and the refusal leaves the file untouched"
    );
}

#[test]
fn an_unreadable_shared_file_is_refused_rather_than_replaced() {
    let dir = tmp("garbage");
    let root = root_with_stdlib(&dir);
    fs::write(root.join("dist/hash/fonts.satysfi-hash"), "this is not json").unwrap();

    let err = install(&font_package(&dir, "fonts-a", "FaceA"), &root, false)
        .expect_err("overwriting it would lose whatever it holds");
    assert!(
        matches!(err, sg::Error::HashFile { .. }),
        "expected a hash-file error, got {err}"
    );
}

#[test]
fn a_root_with_no_hash_file_yet_gets_one() {
    let dir = tmp("fresh");
    let root = dir.join("root");
    fs::create_dir_all(root.join("dist/fonts")).unwrap();

    install(&font_package(&dir, "fonts-a", "FaceA"), &root, false).unwrap();

    assert_eq!(keys(&root), ["FaceA"]);
}

#[test]
fn uninstalling_the_last_contributor_removes_the_file() {
    let dir = tmp("last");
    let root = dir.join("root");
    fs::create_dir_all(root.join("dist/fonts")).unwrap();
    install(&font_package(&dir, "fonts-a", "FaceA"), &root, false).unwrap();

    sg::ops::uninstall::uninstall(
        "fonts-a",
        &sg::ops::uninstall::RootOptions {
            dest: Some(root.clone()),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        !root.join("dist/hash/fonts.satysfi-hash").exists(),
        "an empty shared file is not worth keeping"
    );
}
