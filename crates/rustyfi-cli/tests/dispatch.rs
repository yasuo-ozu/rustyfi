//! Chimera CLI dispatch tests (plan §1/§4/§9): drive the *built* binary
//! under its three personalities by overriding `argv[0]` (`Command::arg0` on
//! unix, and via a real hardlink alias for the `multicall install` helper),
//! plus one install→loader-resolves round trip proving the installer's
//! materialised layout matches `rustyfi_loader`'s `@require:` contract.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// The built `rustyfi` binary (cargo provides this env var to the
/// integration tests of the crate that defines the `[[bin]]`).
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-cli-dispatch-{tag}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        n
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Run `bin()` with an overridden `argv[0]` basename (multicall dispatch key).
#[cfg(unix)]
fn run_as(arg0: &str, args: &[&str]) -> std::process::Output {
    use std::os::unix::process::CommandExt as _;
    Command::new(bin())
        .arg0(arg0)
        .args(args)
        .output()
        .expect("spawn binary")
}

#[cfg(unix)]
#[test]
fn argv0_rustyfi_is_compiler_and_package_manager() {
    let out = run_as("rustyfi", &["--help"]);
    assert!(out.status.success(), "rustyfi --help should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Compile a SATySFi"),
        "rustyfi --help should describe the compiler:\n{stdout}"
    );
    // Renaming the binary merged the two personalities. There used to be a
    // compile-only `rustyfi` beside a full `rustyfi-rust`, and this test
    // asserted the compiler help did NOT mention the package manager; with one
    // name there is one personality, and it carries both.
    assert!(
        stdout.contains("satyrographos"),
        "rustyfi --help should offer the package manager:\n{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn argv0_satyrographos_is_package_manager() {
    let root = tmpdir("sg-personality");
    let out = run_as("satyrographos", &["list", "--dest", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "satyrographos list should succeed on empty root"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("(no packages installed)"),
        "expected empty-list message:\n{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn satyrographos_subcommand_form() {
    // The same package-manager entry point, reached as the nested subcommand
    // under the `rustyfi` personality.
    let root = tmpdir("sg-subcommand");
    let out = run_as(
        "rustyfi",
        &["satyrographos", "list", "--dest", root.to_str().unwrap()],
    );
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("(no packages installed)"));
}

#[cfg(unix)]
#[test]
fn bare_rustyfi_without_input_is_usage_error() {
    let out = run_as("rustyfi", &[]);
    assert!(!out.status.success(), "bare invocation must fail");
    assert_eq!(out.status.code(), Some(2), "clap usage error is exit 2");
}

#[cfg(unix)]
#[test]
fn multicall_install_creates_working_aliases() {
    use std::os::unix::process::CommandExt as _;

    let dir = tmpdir("aliases");
    let out = Command::new(bin())
        .args(["multicall", "install", "--dir", dir.to_str().unwrap()])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "multicall install should succeed");

    let rustyfi_alias = dir.join("rustyfi");
    let satyro_alias = dir.join("satyrographos");
    assert!(rustyfi_alias.exists(), "rustyfi alias created");
    assert!(satyro_alias.exists(), "satyrographos alias created");

    // Invoking the real alias file (its own basename drives dispatch).
    let root = tmpdir("aliases-root");
    let out = Command::new(&satyro_alias)
        .arg0("satyrographos") // basename is already satyrographos; explicit for clarity
        .args(["list", "--dest", root.to_str().unwrap()])
        .output()
        .expect("spawn alias");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("(no packages installed)"));

    // Re-running is idempotent (targets already point at this exe).
    let out = Command::new(bin())
        .args(["multicall", "install", "--dir", dir.to_str().unwrap()])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "re-install of same aliases is idempotent"
    );
}

/// Phase-2 manifest mode (plan §5.3, §8): `satyrographos install` with no
/// PATH locates `Satyrfile.toml` by upward search from the process's working
/// directory, reconciles it, and writes `Satyrfile.lock`. A second run with
/// nothing changed reports every entry as `unchanged`.
#[cfg(unix)]
#[test]
fn manifest_mode_install_reconciles_and_writes_lock() {
    use std::os::unix::process::CommandExt as _;
    let work = tmpdir("manifest-mode");

    // A vendored package source.
    let pkg = work.join("vendor/mylib");
    std::fs::create_dir_all(pkg.join("packages")).unwrap();
    std::fs::write(
        pkg.join("rustyfi-package.toml"),
        "[package]\n\
         name = \"mylib\"\n\
         version = \"1.0.0\"\n\
         rustyfi-version-compat = \">=0.0.6, <0.1\"\n\
         \n\
         [[files]]\n\
         kind = \"package-dir\"\n\
         src = \"packages\"\n",
    )
    .unwrap();
    std::fs::write(pkg.join("packages/mylib.satyh"), "let mylib = 1\n").unwrap();

    // The project directory with a Satyrfile.toml (the process cwd for the run).
    let proj = work.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("Satyrfile.toml"),
        "[[library]]\nname = \"mylib\"\nsource = { path = \"../vendor/mylib\" }\n",
    )
    .unwrap();

    let root = work.join("root");
    let run = |extra_help: &str| {
        Command::new(bin())
            .arg0("satyrographos")
            .current_dir(&proj)
            .args(["install", "--dest", root.to_str().unwrap()])
            .output()
            .unwrap_or_else(|e| panic!("spawn ({extra_help}): {e}"))
    };

    let out = run("first");
    assert!(out.status.success(), "manifest install should succeed");
    assert!(root.join("dist/packages/mylib/mylib.satyh").is_file());
    assert!(proj.join("Satyrfile.lock").is_file(), "lockfile written");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("installed mylib"), "{stdout}");

    // Second run: nothing changed → skipped.
    let out = run("second");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("unchanged mylib"), "{stdout}");

    let _ = std::fs::remove_dir_all(&work);
}

/// Manifest mode with no discoverable `Satyrfile.toml` exits `3` (nothing to
/// operate on).
#[cfg(unix)]
#[test]
fn manifest_mode_without_satyrfile_exits_3() {
    use std::os::unix::process::CommandExt as _;
    let empty = tmpdir("no-satyrfile");
    let root = tmpdir("no-satyrfile-root");
    let out = Command::new(bin())
        .arg0("satyrographos")
        .current_dir(&empty)
        .args(["install", "--dest", root.to_str().unwrap()])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(3), "no Satyrfile → exit 3");
}

/// End-to-end phase-1 contract: install a tiny package into a temp root
/// (through the library API the CLI calls into), then load a document that
/// `@require:`s it against that same root — proving the installer's nested
/// `dist/packages/<name>/<name>.satyh` layout is exactly what the loader's
/// third resolver candidate (plan §3) finds.
#[test]
fn install_then_loader_resolves_the_package() {
    let work = tmpdir("roundtrip");

    // A minimal manifest package declaring a library `greetlib`.
    let src = work.join("src");
    std::fs::create_dir_all(src.join("packages")).unwrap();
    std::fs::write(
        src.join("rustyfi-package.toml"),
        "[package]\n\
         name = \"greetlib\"\n\
         version = \"1.0.0\"\n\
         rustyfi-version-compat = \">=0.0.6, <0.1\"\n\
         \n\
         [[files]]\n\
         kind = \"package-dir\"\n\
         src = \"packages\"\n",
    )
    .unwrap();
    // A library file: bindings only, no `in ...` body.
    std::fs::write(
        src.join("packages/greetlib.satyh"),
        "% library: bindings only\nlet greeting-word = `Hello`\n",
    )
    .unwrap();

    let root = work.join("root");
    rustyfi_satyrographos::install(
        &src,
        &rustyfi_satyrographos::InstallOptions {
            dest: Some(root.clone()),
            ..Default::default()
        },
    )
    .expect("install ok");

    // Sanity: materialised at the nested layout.
    assert!(root.join("dist/packages/greetlib/greetlib.satyh").is_file());

    // A document that requires the just-installed package.
    let doc = work.join("doc.saty");
    std::fs::write(&doc, "@require: greetlib\ndocument (||) '<>\n").unwrap();

    let program = rustyfi_loader::load(
        &doc,
        &rustyfi_loader::LoadOptions {
            lib_root: Some(root.clone()),
            ..Default::default()
        },
    )
    .expect("loader must resolve @require: greetlib against the install root");

    // The loaded program contains the library plus the entry document.
    let names: Vec<String> = program
        .files
        .iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().any(|n| n == "greetlib.satyh"),
        "loaded files should include the installed library: {names:?}"
    );
    assert!(names.iter().any(|n| n == "doc.saty"));

    let _ = std::fs::remove_dir_all(&work);
}
