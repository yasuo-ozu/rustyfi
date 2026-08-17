//! `rustyfi-deps.yaml` decoder — Ld3b-1 (Axis B increment Ld3b, §3.1/§5.1 of
//! `/home/yasuo/.claude/jobs/a7244c0b/tmp/axis-b-ld3b.md`). Transcribed from
//! `saphe-split @ b836d512689248d18970674021ecaca409e0d897`,
//! `src/frontend/depsConfig.ml` (decoder) +
//! `src-common/envelopeSystemBase.ml:89-107` (record shapes) +
//! `src/frontend/configUtil.ml` / `src-common/commonUtil.ml` (field
//! validators) + `src-util/absPath.ml` (absolute-path normalization).
//!
//! This is the compiler-side **decoder only**: nothing in this crate calls
//! [`load`] yet (that wiring — the envelope-graph resolver and `open_doc`'s
//! deps gate — is Ld3b-2). `#![allow(dead_code)]` at the top of this module
//! reflects exactly that; every item here is exercised by this module's own
//! unit tests.
//!
//! `rustyfi-deps.yaml` is what `saphe` *writes* before invoking `rustyfi
//! build --deps <path>`; it is gitignored everywhere upstream (no in-tree
//! fixture exists), so this module's shape is confirmed only by upstream's
//! encoder/decoder pair agreeing field-for-field (`src-saphe/depsConfig.ml`
//! vs. `src/frontend/depsConfig.ml`) — see the Ld3b spec §0.2. The future
//! `rustyfi-saphe` crate (plan Sa1) exchanges this same file format and MUST
//! use the same YAML crate as this module (`serde_yaml`, see this crate's
//! `Cargo.toml`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::LoadError;

/// Decoded `rustyfi-deps.yaml` — upstream `depsConfig.ml:32-37`
/// (`deps_config_decoder`) / `envelopeSystemBase.ml:103-107` (`deps_config`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DepsConfig {
    pub envelopes: Vec<EnvelopeSpec>,
    /// Upstream field `explicit_dependencies`, YAML key `dependencies` — the
    /// document/target's own direct dependencies.
    pub explicit_dependencies: Vec<EnvelopeDependency>,
    /// Upstream `explicit_test_dependencies`, YAML key `test_dependencies`.
    pub explicit_test_dependencies: Vec<EnvelopeDependency>,
}

/// One envelope (package) entry in the deps config —
/// `envelopeSystemBase.ml:95-100` (`envelope_spec`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvelopeSpec {
    /// Arbitrary string (`depsConfig.ml:11-14`: "Envelope names can be
    /// arbitrary strings, even ones that include slashes").
    pub name: String,
    /// Absolute path to the envelope's own `rustyfi-envelope.yaml`
    /// (`depsConfig.ml:26` via `abs_path_decoder`, `configUtil.ml:28-33`) —
    /// not its directory.
    pub path: PathBuf,
    pub dependencies: Vec<EnvelopeDependency>,
    pub test_only: bool,
}

/// One dependency edge — `envelopeSystemBase.ml:89-92` (`envelope_dependency`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvelopeDependency {
    pub name: String,
    /// Validated uppercased identifier (`commonUtil.ml:6-9`): first char
    /// ASCII `A-Z`, rest `-`/ASCII-alphanumeric.
    pub used_as: String,
}

// -- raw serde layer (private): field names ARE the YAML keys --

#[derive(serde::Deserialize)]
struct DepsConfigRaw {
    envelopes: Vec<EnvelopeSpecRaw>,
    dependencies: Vec<EnvelopeDependencyRaw>,
    test_dependencies: Vec<EnvelopeDependencyRaw>,
}

#[derive(serde::Deserialize)]
struct EnvelopeSpecRaw {
    name: String,
    /// Validated absolute in the convert pass below (`parse_abs_path`).
    path: String,
    dependencies: Vec<EnvelopeDependencyRaw>,
    test_only: bool,
}

#[derive(serde::Deserialize)]
struct EnvelopeDependencyRaw {
    name: String,
    used_as: String,
}

/// Load + decode + validate a `rustyfi-deps.yaml`. Errors:
/// [`LoadError::DepsConfigNotFound`] (read failure — upstream
/// `depsConfig.ml:40-45`), [`LoadError::DepsConfigDecode`] (YAML/shape/
/// validation failure — upstream `DepsConfigError`, `depsConfig.ml:46-47`).
pub(crate) fn load(path: &Path) -> Result<DepsConfig, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::DepsConfigNotFound {
        path: path.to_path_buf(),
        source,
    })?;
    decode(&text).map_err(|message| LoadError::DepsConfigDecode {
        path: path.to_path_buf(),
        message,
    })
}

/// `used_as → envelope name`, from a dependency list — upstream
/// `main.ml:103-107` (`make_used_as_map`). A **later** duplicate `used_as`
/// wins (upstream folds `ModuleNameMap.add` left-to-right over the list,
/// and a later `add` for the same key replaces the earlier binding);
/// mirrored here by a plain in-order `HashMap::insert`, which has the same
/// last-write-wins behavior.
pub(crate) fn make_used_as_map(deps: &[EnvelopeDependency]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for dep in deps {
        map.insert(dep.used_as.clone(), dep.name.clone());
    }
    map
}

fn decode(text: &str) -> Result<DepsConfig, String> {
    let raw: DepsConfigRaw = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    convert(raw)
}

fn convert(raw: DepsConfigRaw) -> Result<DepsConfig, String> {
    let envelopes = raw
        .envelopes
        .into_iter()
        .enumerate()
        .map(|(i, spec)| convert_envelope_spec(spec, &format!("envelopes.[{i}]")))
        .collect::<Result<Vec<_>, _>>()?;
    let explicit_dependencies = convert_dependency_list(raw.dependencies, "dependencies")?;
    let explicit_test_dependencies =
        convert_dependency_list(raw.test_dependencies, "test_dependencies")?;
    Ok(DepsConfig {
        envelopes,
        explicit_dependencies,
        explicit_test_dependencies,
    })
}

fn convert_envelope_spec(raw: EnvelopeSpecRaw, ctx: &str) -> Result<EnvelopeSpec, String> {
    let path = parse_abs_path(&raw.path)
        .ok_or_else(|| format!("{ctx}.path: not an absolute path: `{}`", raw.path))?;
    let dependencies = convert_dependency_list(raw.dependencies, &format!("{ctx}.dependencies"))?;
    Ok(EnvelopeSpec {
        name: raw.name,
        path,
        dependencies,
        test_only: raw.test_only,
    })
}

fn convert_dependency_list(
    raw: Vec<EnvelopeDependencyRaw>,
    ctx: &str,
) -> Result<Vec<EnvelopeDependency>, String> {
    raw.into_iter()
        .enumerate()
        .map(|(i, dep)| convert_dependency(dep, &format!("{ctx}.[{i}]")))
        .collect()
}

fn convert_dependency(raw: EnvelopeDependencyRaw, ctx: &str) -> Result<EnvelopeDependency, String> {
    if !is_uppercased_identifier(&raw.used_as) {
        return Err(format!(
            "{ctx}.used_as: not an uppercased identifier: `{}`",
            raw.used_as
        ));
    }
    Ok(EnvelopeDependency {
        name: raw.name,
        used_as: raw.used_as,
    })
}

/// `commonUtil.ml:6-9` (`is_uppercased_identifier`): non-empty, first char
/// ASCII uppercase, every remaining char is `-` or ASCII alphanumeric
/// (`is_middle_char`, `commonUtil.ml:1-2`).
fn is_uppercased_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c0) if c0.is_ascii_uppercase() => chars.all(|c| c == '-' || c.is_ascii_alphanumeric()),
        _ => false,
    }
}

/// `absPath.ml:81-94` (`of_string`) + `:68-78` (`normalize`): the string
/// must start with `/`; the remainder is split on `/` into components,
/// where `.` and the empty component are dropped (`Current`), `..` pops the
/// last kept component (`Parent` — popping past the root, i.e. an empty
/// accumulator, is an error, mirroring `Alist.chop_last` on `[]` returning
/// `None`), and anything else is kept verbatim (`Component`).
fn parse_abs_path(s: &str) -> Option<PathBuf> {
    let rest = s.strip_prefix('/')?;
    let mut components: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            other => components.push(other),
        }
    }
    let mut path = PathBuf::from("/");
    path.extend(components);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `content` to a unique file in the OS temp dir; removed on drop.
    /// Mirrors `tests/loader.rs`'s `TempDir` helper, scaled down to a single
    /// file since `deps.rs`'s unit tests never need a directory tree (deps
    /// configs reference envelopes by absolute *path string*, not by
    /// reading through them — Ld3b-1 is decode-only).
    struct TempFile(PathBuf);

    impl TempFile {
        fn write(tag: &str, content: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rustyfi-loader-deps-test-{tag}-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                n
            ));
            std::fs::write(&path, content).expect("write temp deps config");
            TempFile(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    const DEPS_MINIMAL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v01x/deps/deps-minimal.yaml.tpl"
    ));
    const DEPS_DEMO_SHAPED: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v01x/deps/deps-demo-shaped.yaml.tpl"
    ));

    /// d1: the empty-graph template, via the real (file-reading) `load`.
    #[test]
    fn deps_minimal_decodes() {
        let f = TempFile::write("minimal", DEPS_MINIMAL);
        let cfg = load(&f.0).expect("minimal deps config decodes");
        assert!(cfg.envelopes.is_empty());
        assert!(cfg.explicit_dependencies.is_empty());
        assert!(cfg.explicit_test_dependencies.is_empty());
    }

    /// d2: the demo-lock-shaped 3-envelope template (stdlib / math /
    /// tabular), via `load`, plus `make_used_as_map`'s contents.
    #[test]
    fn deps_demo_shaped_decodes_and_used_as_map() {
        let rendered = DEPS_DEMO_SHAPED.replace("{{ROOT}}", "/synthetic-root");
        let f = TempFile::write("demo-shaped", &rendered);
        let cfg = load(&f.0).expect("demo-shaped deps config decodes");

        assert_eq!(cfg.envelopes.len(), 3);
        let stdlib = &cfg.envelopes[0];
        assert_eq!(
            stdlib.name,
            "registered.6f2b80e9bb7c4e8af2104999fc25dbb3.stdlib.0.0.1"
        );
        assert_eq!(
            stdlib.path,
            PathBuf::from("/synthetic-root/store/stdlib.0.0.1/rustyfi-envelope.yaml")
        );
        assert!(stdlib.dependencies.is_empty());
        assert!(!stdlib.test_only);

        let math = &cfg.envelopes[1];
        assert_eq!(math.dependencies.len(), 1);
        assert_eq!(math.dependencies[0].used_as, "Stdlib");

        assert_eq!(cfg.explicit_dependencies.len(), 3);
        let map = make_used_as_map(&cfg.explicit_dependencies);
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("Stdlib").map(String::as_str),
            Some("registered.6f2b80e9bb7c4e8af2104999fc25dbb3.stdlib.0.0.1")
        );
        assert_eq!(
            map.get("Math").map(String::as_str),
            Some("registered.6f2b80e9bb7c4e8af2104999fc25dbb3.math.0.0.1")
        );
        assert_eq!(
            map.get("Tabular").map(String::as_str),
            Some("registered.6f2b80e9bb7c4e8af2104999fc25dbb3.tabular.0.0.1")
        );
        assert!(cfg.explicit_test_dependencies.is_empty());
    }

    /// `make_used_as_map`: a later duplicate `used_as` wins (upstream's
    /// left-folded `ModuleNameMap.add`, `main.ml:103-107`).
    #[test]
    fn make_used_as_map_later_duplicate_wins() {
        let deps = vec![
            EnvelopeDependency {
                name: "pkg.a".into(),
                used_as: "Foo".into(),
            },
            EnvelopeDependency {
                name: "pkg.b".into(),
                used_as: "Foo".into(),
            },
        ];
        let map = make_used_as_map(&deps);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("Foo").map(String::as_str), Some("pkg.b"));
    }

    /// d4: `test_dependencies` absent entirely is an error — every one of
    /// the three top-level keys is `get` (required), not `get_or_else`.
    #[test]
    fn deps_missing_test_dependencies_errors() {
        let f = TempFile::write("missing-key", "envelopes: []\ndependencies: []\n");
        let err = load(&f.0).expect_err("missing test_dependencies must error");
        assert!(matches!(err, LoadError::DepsConfigDecode { .. }));
    }

    /// d3: a relative `envelopes[].path` is rejected (`abs_path_decoder`).
    #[test]
    fn deps_negative_relative_envelope_path() {
        let f = TempFile::write(
            "rel-path",
            "envelopes:
- name: pkg
  path: \"not/absolute\"
  dependencies: []
  test_only: false
dependencies: []
test_dependencies: []
",
        );
        let err = load(&f.0).expect_err("relative envelope path must error");
        match err {
            LoadError::DepsConfigDecode { message, .. } => {
                assert!(message.contains("envelopes.[0].path"), "{message}");
            }
            other => panic!("expected DepsConfigDecode, got {other:?}"),
        }
    }

    /// d3: `..` walking past the root of an absolute path is an error
    /// (`Alist.chop_last` on an empty accumulator, `absPath.ml:68-78`).
    #[test]
    fn deps_negative_dotdot_past_root() {
        let f = TempFile::write(
            "dotdot-root",
            "envelopes:
- name: pkg
  path: \"/a/../../b\"
  dependencies: []
  test_only: false
dependencies: []
test_dependencies: []
",
        );
        let err = load(&f.0).expect_err("`..` past root must error");
        assert!(matches!(err, LoadError::DepsConfigDecode { .. }));
    }

    /// `..`/`.`/empty-component normalization succeeds and collapses, when
    /// it does not walk past the root.
    #[test]
    fn deps_abs_path_normalizes() {
        assert_eq!(
            parse_abs_path("/a/./b/../c//d"),
            Some(PathBuf::from("/a/c/d"))
        );
        assert_eq!(parse_abs_path("/"), Some(PathBuf::from("/")));
        assert_eq!(parse_abs_path("relative"), None);
        assert_eq!(parse_abs_path("/a/.."), Some(PathBuf::from("/")));
        assert_eq!(parse_abs_path("/.."), None);
    }

    /// d3: a lowercase `used_as` is rejected (`is_uppercased_identifier`).
    #[test]
    fn deps_negative_lowercase_used_as() {
        let f = TempFile::write(
            "lowercase-used-as",
            "envelopes: []
dependencies:
- name: pkg
  used_as: stdlib
test_dependencies: []
",
        );
        let err = load(&f.0).expect_err("lowercase used_as must error");
        match err {
            LoadError::DepsConfigDecode { message, .. } => {
                assert!(message.contains("dependencies.[0].used_as"), "{message}");
            }
            other => panic!("expected DepsConfigDecode, got {other:?}"),
        }
    }

    /// d3: `used_as` with an embedded space is rejected (not `[-A-Za-z0-9]`).
    #[test]
    fn deps_negative_used_as_with_space() {
        let f = TempFile::write(
            "used-as-space",
            "envelopes: []
dependencies:
- name: pkg
  used_as: \"St dlib\"
test_dependencies: []
",
        );
        let err = load(&f.0).expect_err("used_as with a space must error");
        assert!(matches!(err, LoadError::DepsConfigDecode { .. }));
    }

    /// Two `envelopes[].name` entries sharing a name decode successfully —
    /// the *conflict* is a resolver-level error (`EnvelopeNameConflict`,
    /// `closedEnvelopeDependencyResolver.ml:50-51`), which is Ld3b-2
    /// (`closed::sort_envelopes`) territory, not this decoder's.
    #[test]
    fn deps_duplicate_envelope_name_decodes_fine() {
        let f = TempFile::write(
            "dup-name",
            "envelopes:
- name: pkg
  path: \"/a/rustyfi-envelope.yaml\"
  dependencies: []
  test_only: false
- name: pkg
  path: \"/b/rustyfi-envelope.yaml\"
  dependencies: []
  test_only: false
dependencies: []
test_dependencies: []
",
        );
        let cfg = load(&f.0).expect("duplicate envelope names decode fine at Ld3b-1");
        assert_eq!(cfg.envelopes.len(), 2);
    }

    /// Reading a missing file surfaces the read-failure variant, not the
    /// decode one.
    #[test]
    fn deps_file_missing_is_not_found() {
        let path = std::env::temp_dir().join("rustyfi-loader-deps-test-does-not-exist.yaml");
        let err = load(&path).expect_err("missing file must error");
        assert!(matches!(err, LoadError::DepsConfigNotFound { .. }));
    }
}
