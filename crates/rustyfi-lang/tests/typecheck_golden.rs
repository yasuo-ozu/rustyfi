//! Whole-corpus typecheck snapshot: for every `.saty`/`.satyh`/`.satyg`
//! fixture under `crates/*/tests/fixtures/` and every bundled package under
//! `lib-rustyfi/dist/packages/` (loader-merged the same way `stdja` loads, via
//! a synthetic `@require: <pkg>` entry), run parse -> elaborate ->
//! `typecheck_verbose` and record one deterministic line per input:
//! `OK <tag> (<version>) <n-warnings> [<warnings>]`.
//!
//! The point is the STRINGS. A refactor that swaps an `expected`/`found`, drops
//! a `?`, or moves a match arm by one changes an error or warning message
//! somewhere in the corpus while every verdict stays the same — which spot
//! assertions in the unit suite do not notice, and this does.
//!
//! Compared against `snapshots/typecheck_golden.txt`; re-pin with:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p rustyfi-lang --test typecheck_golden
//! ```
//!
//! Inputs that CANNOT typecheck live in `typecheck_known_gaps.rs`, asserted
//! individually with the reason, so this snapshot holds no `ERR` line and means
//! exactly "everything that should typecheck, does".
//!
//! This used to be an `#[ignore]`d harness driven by `scripts/typecheck-golden.sh`,
//! which ran it against two git refs and diffed the outputs. The committed
//! snapshot subsumes that for the case that matters — "did anything move?" —
//! and runs on an ordinary `cargo test`, at the cost of the arbitrary
//! ref-to-ref comparison the script could also do.

use rustyfi_lang::{elaborate, primitives, typecheck, v1};
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedFile, LoadedProgram};
use rustyfi_syntax::RustyfiVersion;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Inputs asserted in `typecheck_known_gaps.rs` instead of here. Keep the two
/// in step: that file fails if any of these leaks back into the snapshot.
const KNOWN_GAPS: &[&str] = &[
    "crates/rustyfi-cli/tests/fixtures/envelopes/doc.saty",
    "crates/rustyfi-cli/tests/fixtures/envelopes/v01-mini.satyh",
    // Mixed-version too, for the same reason, since `@require:` now prefers
    // the entry's own generation: as a 0.1 entry this resolves `list` to the
    // 0.1 corpus, whose `List` has `fold` where the fixture (deliberately
    // written against 0.0.6's) calls `fold-left`. It compiles through the real
    // pipeline in `xver_capstone.rs`, against a 0.0.6-only lib root.
    "crates/rustyfi-cli/tests/fixtures/xver-capstone.saty",
    // These seven COMPILE AND RENDER through the real pipeline; they are
    // excluded only because this harness merges every dependency's prelude
    // itself and elaborates the result under ONE version, so a mixed-version
    // program trips over a 0.1 dependency's labeled optional arguments while
    // "compiled as 0.0.6". Teach the harness to merge per-version and they
    // belong back in the snapshot.
    "crates/rustyfi-cli/tests/fixtures/math-cramped.saty",
    "crates/rustyfi-cli/tests/fixtures/multifile/helpers.satyh",
    "crates/rustyfi-cli/tests/fixtures/v01-footnote-scheme.saty",
    "crates/rustyfi-cli/tests/fixtures/v01-itemize.saty",
    "crates/rustyfi-cli/tests/fixtures/v01-math-full.saty",
    "crates/rustyfi-cli/tests/fixtures/v01-stdja-book.saty",
    "crates/rustyfi-cli/tests/fixtures/v01-stdja-report.saty",
    "crates/rustyfi-cli/tests/fixtures/v01-stdja.saty",
    "crates/rustyfi-cli/tests/fixtures/xver-capstone-helper.satyh",
];

/// This repo's root, resolved relative to this crate's own manifest
/// directory (`crates/rustyfi-lang`) — same convention as
/// `stdlib_tier0.rs`'s `lib_root`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should exist")
}

fn lib_root() -> PathBuf {
    repo_root().join("lib-rustyfi")
}

/// Every `crates/*/tests/fixtures/**/*.saty(h|g)` file, sorted for
/// deterministic output order.
fn fixture_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates_dir = repo_root().join("crates");
    for entry in fs::read_dir(&crates_dir).expect("read crates/") {
        let entry = entry.expect("read_dir entry");
        let fixtures = entry.path().join("tests").join("fixtures");
        if fixtures.is_dir() {
            walk_saty_files(&fixtures, &mut out);
        }
    }
    out.sort();
    out
}

fn walk_saty_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if path.is_dir() {
            walk_saty_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("saty") | Some("satyh") | Some("satyg")
        ) {
            out.push(path);
        }
    }
}

/// Every bundled package name under `lib-rustyfi/dist/packages/` (file stem
/// only — the name `@require:` resolves), sorted for deterministic output
/// order. 29 at the L3 baseline (see the spec's §8.3).
fn package_names() -> Vec<String> {
    let mut out = Vec::new();
    let dir = lib_root().join("dist").join("packages");
    for entry in fs::read_dir(&dir).expect("read dist/packages/") {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("satyh") | Some("satyg")
        ) {
            out.push(
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .expect("package file stem")
                    .to_string(),
            );
        }
    }
    out.sort();
    out
}

/// A uniquely-named temp `.saty` entry file, cleaned up on drop — used only
/// for the synthetic `@require: <pkg> in ()` package-probe entries.
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-typecheck-golden-{tag}-{}-{}.saty",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp golden entry");
        TempDoc(path)
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn as_v006(cst: &LoadedCst) -> &rustyfi_syntax::cst::File {
    match cst {
        LoadedCst::V0_0_6(f) => f,
        LoadedCst::V0_1(_) => unreachable!("as_v006 called on a V0_1-loaded file"),
    }
}

/// Merge a loader-resolved V0.0.6 program's preludes into one synthetic
/// `cst::File` — mirrors `rustyfi-cli`'s `merge_program` /
/// `stdlib_tier0.rs`'s helper of the same name.
fn merge_program_v006(program: LoadedProgram) -> rustyfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(&entry.cst).clone();
    let mut prelude = Vec::new();
    for lib in &files {
        prelude.extend(as_v006(&lib.cst).prelude.clone());
    }
    prelude.extend(entry_cst.prelude);
    rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

fn as_v01(f: &LoadedFile) -> &rustyfi_syntax::cst_v1::FileV1 {
    match &f.cst {
        LoadedCst::V0_1(cst) => cst,
        LoadedCst::V0_0_6(_) => unreachable!("as_v01 called on a V0_0_6-loaded file"),
    }
}

/// V0_1 analogue of `merge_program_v006` — mirrors `lib.rs`'s
/// `compile_document_v1_with_trials` assembly step (lowering each
/// dependency + the entry through `v1::lower`), stopping short of eval.
/// Also returns the X2a `v006_indices` set (mirrors production's — see
/// `elaborate::elaborate_program_with_versions`), so callers can wrap
/// spliced V0_0_6 bindings the same way production does.
fn merge_program_v01(
    files: &[LoadedFile],
) -> Result<(rustyfi_syntax::cst::File, std::collections::HashSet<usize>), v1::lower::LowerError> {
    let (entry, deps) = files
        .split_last()
        .expect("loader always yields at least the entry file");
    let mut prelude = Vec::new();
    let mut v006_indices = std::collections::HashSet::new();
    for dep in deps {
        // X1/X2a (design-cross-version-import.md §5, §"Slice X2 — per-group
        // primitive environment"): mirror production's
        // `compile_document_v1_with_trials` dep loop, which is now a
        // MIXED-version list. A V0_1 dep is lowered as before; a V0_0_6 dep (a
        // 0.0.6-corpus `@require:` target reached under a V0_1 entry) splices
        // its `cst::File.prelude` bindings directly, and its contributed
        // top-level indices are recorded into `v006_indices` — mirroring
        // production's index bookkeeping — so `one_line`'s V0_1 branch can
        // call `elaborate::elaborate_program_with_versions` the same way
        // `compile_document_v1_with_trials` does, keeping the two in sync
        // for a mixed-version fixture's `Ast::VersionScope` shape. Production
        // ALSO runs the forked-name guard on this path, but this
        // typecheck-differential harness only needs to not panic + emit a
        // stable line, so it still splices unconditionally, guard-free. (The
        // V0_0_6-first branch in `one_line` — the 0.0.6 golden lines — never
        // reaches here and is untouched by X1/X2a.)
        match &dep.cst {
            LoadedCst::V0_1(cst) => prelude.extend(v1::lower::lower_file_v1(cst)?),
            LoadedCst::V0_0_6(cst) => {
                let start = prelude.len();
                prelude.extend(cst.prelude.iter().cloned());
                v006_indices.extend(start..prelude.len());
            }
        }
    }
    let entry_cst = as_v01(entry);
    let body = v1::lower::lower_document_v1(entry_cst)?;
    let eoi = match entry_cst {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => unreachable!("lower_document_v1 already rejected a Library entry"),
    };
    Ok((
        rustyfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: Some(rustyfi_syntax::leaf::KwIn(rustyfi_syntax::Span::default())),
            body: Some(body),
            eoi,
        },
        v006_indices,
    ))
}

/// Load + elaborate + `typecheck_verbose` one entry file, trying V0_0_6
/// first and falling back to V0_1 if the V0_0_6 load itself fails (the
/// corpus mixes both generations, e.g. `v01-*.saty` fixtures) — the version
/// used is baked into `tag` so the golden output stays stable across runs
/// regardless of which branch actually fired.
fn one_line(tag: &str, entry: &Path) -> String {
    let opts_006 = LoadOptions {
        lib_root: Some(lib_root()),
        version: RustyfiVersion::V0_0_6,
        ..Default::default()
    };
    match rustyfi_loader::load(entry, &opts_006) {
        Ok(program) => {
            let file = merge_program_v006(program);
            render(tag, "0.0.6", || {
                let env = primitives::base_env();
                let store = rustyfi_lang::symbol::SymbolStore::new();
                let scope = elaborate::Scope::new(&store, env.names());
                let program = elaborate::elaborate_program(&file, &scope)
                    .map_err(|e| format!("elaborate: {e}"))?;
                typecheck::typecheck_verbose(&program).map_err(|e| format!("{e}"))
            })
        }
        Err(v006_err) => {
            let opts_01 = LoadOptions {
                lib_root: Some(lib_root()),
                version: RustyfiVersion::V0_1,
                ..Default::default()
            };
            match rustyfi_loader::load(entry, &opts_01) {
                Ok(program) => match merge_program_v01(&program.files) {
                    Ok((file, v006_indices)) => render(tag, "0.1", || {
                        let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
                        let store = rustyfi_lang::symbol::SymbolStore::new();
                        let scope = elaborate::Scope::new(&store, env.names());
                        let program = elaborate::elaborate_program_with_versions(
                            &file,
                            &scope,
                            &v006_indices,
                            None,
                        )
                        .map_err(|e| format!("elaborate: {e}"))?;
                        typecheck::typecheck_verbose_with_version(&program, RustyfiVersion::V0_1)
                            .map_err(|e| format!("{e}"))
                    }),
                    Err(lower_err) => format!("ERR {tag} (0.1) lower: {lower_err}"),
                },
                // Neither version's loader could even parse/resolve this
                // entry — report the original V0.0.6 load error (stable,
                // version-independent framing).
                Err(_v01_err) => format!("ERR {tag} load: {v006_err}"),
            }
        }
    }
}

/// Make one rendered line machine-independent.
///
/// A few inputs fail in the LOADER, and a loader error's `Display` embeds the
/// absolute path it was resolving. Left alone those lines carry whoever's
/// checkout produced them, which is fine for the ref-to-ref diff the script
/// does (both sides share a root and it cancels) but fatal for a COMMITTED
/// golden file. Also normalize the temp path a package probe's synthetic entry
/// lives at, which carries a pid and a counter.
fn relativize(line: String) -> String {
    let root = format!("{}/", repo_root().display());
    let line = line.replace(&root, "");
    let tmp = format!("{}/", std::env::temp_dir().display());
    match line.find(&tmp) {
        None => line,
        Some(i) => {
            let rest = &line[i + tmp.len()..];
            let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            format!("{}<tmp>/<probe-entry>{}", &line[..i], &rest[end..])
        }
    }
}

fn render(
    tag: &str,
    version_tag: &str,
    f: impl FnOnce() -> Result<Vec<typecheck::MatchWarning>, String>,
) -> String {
    match f() {
        Ok(warnings) => {
            let rendered: Vec<String> = warnings.iter().map(|w| format!("{w:?}")).collect();
            format!(
                "OK {tag} ({version_tag}) {} [{}]",
                warnings.len(),
                rendered.join("; ")
            )
        }
        Err(e) => format!("ERR {tag} ({version_tag}) {e}"),
    }
}

#[test]
fn typecheck_golden() {
    let lines: Vec<String> = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut lines = Vec::new();

            for path in fixture_files() {
                let tag = path
                    .strip_prefix(repo_root())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                // Inputs that cannot typecheck are asserted individually, with
                // the REASON, in `typecheck_known_gaps.rs` — a bare `ERR` line
                // here pins a failure without saying whether it is intended,
                // and renders whatever this harness's own prelude merge
                // produces rather than what the compiler actually reports.
                // What is left is exactly "everything that should typecheck,
                // does", so this snapshot should hold no `ERR` line at all
                // (`known_gaps_are_excluded_from_the_golden_snapshot` checks
                // both halves of that split).
                if KNOWN_GAPS.iter().any(|g| tag == *g) {
                    continue;
                }
                lines.push(one_line(&tag, &path));
            }

            for pkg in package_names() {
                let src = format!("@require: {pkg}\nin\n()");
                let doc = TempDoc::new(&pkg, &src);
                let tag = format!("package:{pkg}");
                lines.push(one_line(&tag, &doc.0));
            }

            let mut lines: Vec<String> = lines.into_iter().map(relativize).collect();
            lines.sort();
            lines
        })
        .expect("spawn big-stack thread")
        .join()
        .expect("golden harness thread panicked");

    let actual = format!("{}\n", lines.join("\n"));
    let path = repo_root().join("crates/rustyfi-lang/tests/snapshots/typecheck_golden.txt");

    if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        fs::create_dir_all(path.parent().expect("snapshots dir")).expect("create snapshots dir");
        fs::write(&path, &actual).expect("write the golden snapshot");
        eprintln!(
            "UPDATE_SNAPSHOTS=1: wrote {} lines to {}",
            lines.len(),
            path.display()
        );
        return;
    }

    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nrun with UPDATE_SNAPSHOTS=1 to create it",
            path.display()
        )
    });
    if actual == expected {
        return;
    }

    // A line-level diff: the whole corpus in one blob is unreadable, and the
    // point of this harness is to name the one input whose error or warning
    // STRING moved.
    let exp: Vec<&str> = expected.lines().collect();
    let act: Vec<&str> = actual.lines().collect();
    let mut report = String::new();
    for tag in tags_of(&exp).union(&tags_of(&act)) {
        let e = exp.iter().find(|l| tag_of(l) == *tag);
        let a = act.iter().find(|l| tag_of(l) == *tag);
        match (e, a) {
            (Some(e), Some(a)) if e != a => {
                report.push_str(&format!("  ~ {tag}\n    was: {e}\n    now: {a}\n"));
            }
            (Some(e), None) => report.push_str(&format!("  - {tag}\n    was: {e}\n")),
            (None, Some(a)) => report.push_str(&format!("  + {tag}\n    now: {a}\n")),
            _ => {}
        }
    }
    panic!(
        "typecheck output changed for {} input(s).\n\n{report}\nIf this is intended, \
         re-pin with:\n  UPDATE_SNAPSHOTS=1 cargo test -p rustyfi-lang --test typecheck_golden\n\
         A file that stops typechecking ENTIRELY belongs in typecheck_known_gaps.rs instead.",
        report
            .lines()
            .filter(|l| l.starts_with("  ~ ") || l.starts_with("  - ") || l.starts_with("  + "))
            .count(),
    );
}

/// The input identity a line is about — everything up to the `(version)` or the
/// `load:` marker. Diffs are matched on this so a changed message is reported
/// as a modification rather than as one removal plus one addition.
fn tag_of(line: &str) -> &str {
    let rest = line
        .strip_prefix("OK ")
        .or_else(|| line.strip_prefix("ERR "))
        .unwrap_or(line);
    rest.split_whitespace().next().unwrap_or(rest)
}

fn tags_of<'a>(lines: &[&'a str]) -> std::collections::BTreeSet<&'a str> {
    lines.iter().map(|l| tag_of(l)).collect()
}
