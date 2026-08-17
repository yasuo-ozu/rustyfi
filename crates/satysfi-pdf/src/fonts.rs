//! Font configuration discovery and resolution (text-rendering plan, Slice
//! 1): reads a small, plain-JSON font configuration and turns it into a
//! [`TtfFontStore`] via its existing [`TtfFontStore::load`], so a document
//! naming a real font renders through [`crate::render_pdf_ttf`] instead of
//! falling back to the base-14 metrics.
//!
//! This module only *resolves configuration into a store*; it does not
//! change how typesetting reaches fonts at all — `set-font-key` already
//! routes a `FontKey` through whatever `&dyn FontMetrics` the caller passed
//! to `compile_document_cst`, so swapping in a [`TtfFontStore`] here is the
//! entire change on the typesetting side.
//!
//! # Why not upstream's `fonts.satysfi-hash`?
//!
//! SATySFi v0.0.6 keys font selection through two files under
//! `<runtime>/dist/hash/`: `fonts.satysfi-hash` maps a font *abbrev* to a
//! font file, and `default-font.satysfi-hash` maps a *script* to `{
//! font-name = abbrev; ratio; rising }`. The latter is already plain JSON,
//! but the former uses Yojson's non-standard `<Variant: {...}>` syntax
//! (e.g. `<Single: {"src": "..."}>`), which no standard JSON parser
//! (`serde_json` included) accepts. Rather than hand-roll a Yojson reader
//! for one milestone slice, this port defines its own plain-JSON schema at
//! the same filenames — so the directory layout (`dist/hash/*.satysfi-hash`)
//! stays familiar to anyone who has seen upstream's runtime tree — and
//! documents the reshaping here instead of silently diverging. See the
//! plan's Risks section ("`.satysfi-hash` is not standard JSON").
//!
//! `serde`/`serde_json` were chosen over a tiny hand-rolled parser because
//! the schema below is a plain, statically-shaped JSON object (no variant
//! syntax to special-case) — exactly what `#[derive(Deserialize)]` is for —
//! and `serde` is already a workspace dependency; reaching for a hand parser
//! here would just be reimplementing JSON string/number/object escaping for
//! no benefit.
//!
//! # `fonts.satysfi-hash` (this port's schema)
//!
//! A JSON object mapping an arbitrary *abbrev* name to a font source:
//!
//! ```json
//! { "lmroman":  { "src": "dist/fonts/lmroman10-regular.otf" },
//!   "ipaexm":   { "src": "dist/fonts/ipaexm.ttf" },
//!   "somettc":  { "src": "dist/fonts/foo.ttc", "index": 0 } }
//! ```
//!
//! `src` is a path to a font file: resolved relative to the *font root*
//! (the directory under which `dist/hash/fonts.satysfi-hash` was found)
//! when relative, or used as-is when absolute. `index`, present only for a
//! TrueType Collection (`.ttc`) member, mirrors v0.0.6's
//! `FontAccess.Collection` — but only `index: 0` can actually be *loaded*
//! today: [`TtfFontStore::load`] always parses face 0 of whatever file it is
//! given, so a non-zero index is accepted by the schema (forward
//! compatibility for when the store grows real TTC-index support) but
//! rejected with a clear [`FontConfigError::UnsupportedCollectionIndex`] at
//! [`FontRegistry::build_store`] time — never silently loading the wrong
//! face.
//!
//! # `default-font.satysfi-hash` (port-specific; not upstream's schema)
//!
//! Upstream's `default-font.satysfi-hash` maps *script* to a font selection
//! (with `ratio`/`rising` scaling knobs) — that needs script segmentation
//! this milestone does not have yet (see the plan's "Full roadmap" §3/§4).
//! Slice 1 only needs to seed the three faces the base-14 provider already
//! has (`FontKey(0/1/2)` = regular/bold/oblique, `base14.rs`), so this port
//! defines a much smaller, unrelated schema at the same filename:
//!
//! ```json
//! { "regular": "lmroman", "bold": "lmroman-bold", "oblique": "lmroman-oblique" }
//! ```
//!
//! Only `regular` is required; `bold`/`oblique` default to `regular`'s own
//! abbrev when omitted, so [`FontRegistry::build_store`] calls
//! [`TtfFontStore::load`] with `None` for the missing slot(s) — exactly
//! `TtfFontStore`'s own bold/oblique-falls-back-to-regular behavior, rather
//! than this module loading the regular face's bytes a second time under a
//! different slot.
//!
//! # Discovery and error handling
//!
//! See [`FontRegistry::discover`] for the full precedence chain. In short: a
//! missing configuration (nothing found at the location being examined) is
//! "nothing configured" and resolves to `Ok(None)`, so a caller with no
//! fonts set up anywhere renders exactly as before this module existed. Once
//! *something* is found, though, further problems (malformed JSON, a
//! default-face abbrev that isn't defined, a font file that fails to load)
//! are real errors (`Err`) rather than a silent fall-back to base-14 —
//! deliberately, so a broken font configuration is never confused with "no
//! font configuration".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::ttf::{FontError, TtfFontStore};

/// One font source, as resolved from `fonts.satysfi-hash` (`src` already
/// joined against the font root) or synthesized from a `--font`/
/// `--font-bold`/`--font-oblique` CLI flag.
///
/// Mirrors v0.0.6's `FontAccess` (`loadFont.ml:43-47`); see the module docs
/// for why `Collection`'s index is currently limited to `0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSource {
    /// A plain font file (TrueType, OpenType, ...).
    Single(PathBuf),
    /// One face of a TrueType Collection (`.ttc`), by index.
    Collection(PathBuf, u32),
}

impl FontSource {
    /// The underlying file path, regardless of variant.
    pub fn path(&self) -> &Path {
        match self {
            FontSource::Single(path) => path,
            FontSource::Collection(path, _) => path,
        }
    }
}

/// A resolved `abbrev -> font file` mapping plus the three seeded default
/// faces (`FontKey(0/1/2)` = regular/bold/oblique), ready to become a
/// [`TtfFontStore`] via [`FontRegistry::build_store`].
///
/// Keeping the full abbrev map around (not just the three resolved paths)
/// is deliberate scope for the immediate follow-on the plan describes: a
/// real `set-font` primitive that resolves an *arbitrary* abbrev to a
/// newly-allocated `FontKey` beyond these three seeded slots. Slice 1 does
/// not add that primitive, but this shape does not have to be revisited to
/// support it later.
#[derive(Debug, Clone)]
pub struct FontRegistry {
    faces: BTreeMap<String, FontSource>,
    /// `[regular, bold, oblique]` abbrevs. `bold`/`oblique` equal `regular`
    /// verbatim when the config left them unset, which `build_store` uses
    /// to decide when to pass `None` (rather than resolving and loading the
    /// same file a second time) to `TtfFontStore::load`.
    default_faces: [String; 3],
}

/// Config-less one-off face selection (`--font`/`--font-bold`/
/// `--font-oblique`): the highest-precedence source in
/// [`FontRegistry::discover`], bypassing `fonts.satysfi-hash` entirely.
#[derive(Debug, Clone, Default)]
pub struct FontFlags {
    pub regular: Option<PathBuf>,
    pub bold: Option<PathBuf>,
    pub oblique: Option<PathBuf>,
}

impl FontFlags {
    fn is_empty(&self) -> bool {
        self.regular.is_none() && self.bold.is_none() && self.oblique.is_none()
    }
}

/// Errors from discovering or resolving a font configuration. Distinct from
/// [`FontError`] (which is about a font *file* failing to load/parse): this
/// type is about the *configuration* pointing at a bad state in the first
/// place (malformed JSON, a dangling abbrev reference, an unsupported TTC
/// index, ...). [`FontError`]s that occur while actually loading a resolved
/// path are wrapped via `Font`.
#[derive(Debug, thiserror::Error)]
pub enum FontConfigError {
    #[error("failed to read font config {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse font config {path} as JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "{path}: default-face {face:?} names abbrev {abbrev:?}, which is not \
         defined in fonts.satysfi-hash"
    )]
    UnknownAbbrev {
        path: PathBuf,
        face: &'static str,
        abbrev: String,
    },
    #[error(
        "font abbrev {abbrev:?} names a font collection at TTC index {index}; \
         only index 0 can be loaded in this port (TtfFontStore does not yet \
         support selecting a non-zero face of a collection)"
    )]
    UnsupportedCollectionIndex { abbrev: String, index: u32 },
    #[error("--font-bold/--font-oblique require --font (no regular face given)")]
    RegularRequired,
    #[error(transparent)]
    Font(#[from] FontError),
}

/// Raw shape of one entry in `fonts.satysfi-hash` (see module docs). `index`
/// present ⇒ a `.ttc` member.
#[derive(Debug, Deserialize)]
struct RawFontEntry {
    src: PathBuf,
    #[serde(default)]
    index: Option<u32>,
}

/// Raw shape of `default-font.satysfi-hash` (port-specific; see module
/// docs). Only `regular` is required.
#[derive(Debug, Deserialize)]
struct RawDefaultFace {
    regular: String,
    #[serde(default)]
    bold: Option<String>,
    #[serde(default)]
    oblique: Option<String>,
}

/// Synthetic abbrev names for the config-less `--font`/`--font-bold`/
/// `--font-oblique` flags (never emitted by `fonts.satysfi-hash` parsing,
/// so they cannot collide with a real config's abbrevs).
const CLI_REGULAR: &str = "<--font>";
const CLI_BOLD: &str = "<--font-bold>";
const CLI_OBLIQUE: &str = "<--font-oblique>";

impl FontRegistry {
    /// Resolve Slice-1 font configuration, highest precedence first:
    ///
    /// 1. `flags` (`--font`, `+ --font-bold`/`--font-oblique`) — a
    ///    config-less one-off; `regular` is required if either of the other
    ///    two is set (enforced by the CLI's `clap` arg group, and re-checked
    ///    here defensively).
    /// 2. `font_dir` — the resolved font root (the CLI folds its
    ///    `--font-dir` flag and `$SATYSFI_FONT_DIR` into this one
    ///    parameter, mirroring how it resolves `--lib-root`).
    /// 3. `lib_root` — reused as the font root when neither of the above is
    ///    given, so a project that keeps `dist/hash/fonts.satysfi-hash`
    ///    alongside `dist/packages/` needs no extra flag.
    ///
    /// Returns `Ok(None)` when nothing is configured at all: no flags, and
    /// no `dist/hash/fonts.satysfi-hash` under whichever root ends up being
    /// examined (or no root to examine, i.e. both `font_dir` and `lib_root`
    /// are `None`). The caller then keeps today's base-14 path verbatim.
    ///
    /// Once a `fonts.satysfi-hash` *is* found, any further problem (bad
    /// JSON, a missing `default-font.satysfi-hash`, a default-face abbrev
    /// absent from the abbrev map) is `Err`, not `Ok(None)` — a config that
    /// exists but is broken should be reported, not silently swapped for
    /// base-14 (see the module docs).
    pub fn discover(
        lib_root: Option<&Path>,
        font_dir: Option<&Path>,
        flags: &FontFlags,
    ) -> Result<Option<FontRegistry>, FontConfigError> {
        if !flags.is_empty() {
            return Self::from_flags(flags).map(Some);
        }

        let Some(root) = font_dir.or(lib_root) else {
            return Ok(None);
        };

        let hash_dir = root.join("dist").join("hash");
        let fonts_path = hash_dir.join("fonts.satysfi-hash");
        let fonts_bytes = match std::fs::read(&fonts_path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(FontConfigError::Io {
                    path: fonts_path,
                    source,
                })
            }
        };
        let raw: BTreeMap<String, RawFontEntry> =
            serde_json::from_slice(&fonts_bytes).map_err(|source| FontConfigError::Json {
                path: fonts_path.clone(),
                source,
            })?;
        let faces: BTreeMap<String, FontSource> = raw
            .into_iter()
            .map(|(abbrev, entry)| {
                // `Path::join` discards `root` entirely when `entry.src` is
                // itself absolute, so this handles both relative and
                // absolute `src` values correctly in one call.
                let resolved = root.join(&entry.src);
                let source = match entry.index {
                    Some(index) => FontSource::Collection(resolved, index),
                    None => FontSource::Single(resolved),
                };
                (abbrev, source)
            })
            .collect();

        let default_path = hash_dir.join("default-font.satysfi-hash");
        let default_bytes = std::fs::read(&default_path).map_err(|source| FontConfigError::Io {
            path: default_path.clone(),
            source,
        })?;
        let raw_default: RawDefaultFace =
            serde_json::from_slice(&default_bytes).map_err(|source| FontConfigError::Json {
                path: default_path.clone(),
                source,
            })?;

        let regular = raw_default.regular;
        let bold = raw_default.bold.unwrap_or_else(|| regular.clone());
        let oblique = raw_default.oblique.unwrap_or_else(|| regular.clone());

        for (face, abbrev) in [
            ("regular", &regular),
            ("bold", &bold),
            ("oblique", &oblique),
        ] {
            if !faces.contains_key(abbrev) {
                return Err(FontConfigError::UnknownAbbrev {
                    path: default_path,
                    face,
                    abbrev: abbrev.clone(),
                });
            }
        }

        Ok(Some(FontRegistry {
            faces,
            default_faces: [regular, bold, oblique],
        }))
    }

    /// Synthesize a registry directly from `--font`/`--font-bold`/
    /// `--font-oblique`, with no `fonts.satysfi-hash` involved at all.
    fn from_flags(flags: &FontFlags) -> Result<FontRegistry, FontConfigError> {
        let Some(regular) = &flags.regular else {
            return Err(FontConfigError::RegularRequired);
        };
        let mut faces = BTreeMap::new();
        faces.insert(CLI_REGULAR.to_string(), FontSource::Single(regular.clone()));
        let bold_abbrev = match &flags.bold {
            Some(path) => {
                faces.insert(CLI_BOLD.to_string(), FontSource::Single(path.clone()));
                CLI_BOLD.to_string()
            }
            None => CLI_REGULAR.to_string(),
        };
        let oblique_abbrev = match &flags.oblique {
            Some(path) => {
                faces.insert(CLI_OBLIQUE.to_string(), FontSource::Single(path.clone()));
                CLI_OBLIQUE.to_string()
            }
            None => CLI_REGULAR.to_string(),
        };
        Ok(FontRegistry {
            faces,
            default_faces: [CLI_REGULAR.to_string(), bold_abbrev, oblique_abbrev],
        })
    }

    /// Resolve the three seeded default faces and build a [`TtfFontStore`],
    /// via the *existing* [`TtfFontStore::load`] — whose bold/oblique
    /// fall-back-to-regular behavior is exactly what an unconfigured
    /// bold/oblique slot should do, so this passes `None` for a slot whose
    /// abbrev equals the regular one rather than re-resolving and loading
    /// the same file under a second slot.
    pub fn build_store(&self) -> Result<TtfFontStore, FontConfigError> {
        let regular = self.resolve(&self.default_faces[0])?;
        let bold = if self.default_faces[1] == self.default_faces[0] {
            None
        } else {
            Some(self.resolve(&self.default_faces[1])?)
        };
        let oblique = if self.default_faces[2] == self.default_faces[0] {
            None
        } else {
            Some(self.resolve(&self.default_faces[2])?)
        };
        Ok(TtfFontStore::load(
            &regular,
            bold.as_deref(),
            oblique.as_deref(),
        )?)
    }

    /// Resolve `abbrev` to a loadable file path.
    ///
    /// Panics if `abbrev` is not a key of `self.faces` — an invariant both
    /// `discover` (which validates every `default_faces` abbrev against the
    /// parsed map before returning) and `from_flags` (which always inserts
    /// an abbrev and its face together) maintain by construction, so this
    /// is only ever reached with a valid key.
    fn resolve(&self, abbrev: &str) -> Result<PathBuf, FontConfigError> {
        match self
            .faces
            .get(abbrev)
            .unwrap_or_else(|| panic!("FontRegistry invariant violated: {abbrev:?} unresolved"))
        {
            FontSource::Single(path) => Ok(path.clone()),
            FontSource::Collection(path, 0) => Ok(path.clone()),
            FontSource::Collection(_, index) => Err(FontConfigError::UnsupportedCollectionIndex {
                abbrev: abbrev.to_string(),
                index: *index,
            }),
        }
    }

    /// The abbrev -> font source map, e.g. for a future `set-font` primitive
    /// resolving an abbrev beyond the three seeded default slots.
    pub fn faces(&self) -> &BTreeMap<String, FontSource> {
        &self.faces
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use satysfi_backend::FontMetrics as _;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmpdir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "satysfi-pdf-fonts-test-{tag}-{}-{}-{n}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_hash_dir(root: &Path, fonts_json: &str, default_json: Option<&str>) {
        let hash_dir = root.join("dist/hash");
        std::fs::create_dir_all(&hash_dir).unwrap();
        std::fs::write(hash_dir.join("fonts.satysfi-hash"), fonts_json).unwrap();
        if let Some(default_json) = default_json {
            std::fs::write(hash_dir.join("default-font.satysfi-hash"), default_json).unwrap();
        }
    }

    /// Locate a real TrueType file, exactly like `tests/ttf.rs`'s
    /// `find_regular_font`, for the handful of tests that must actually
    /// call `build_store` successfully (which loads real font bytes).
    /// Gracefully skipped (not failed) when absent, as elsewhere in this
    /// codebase.
    fn find_regular_font() -> Option<PathBuf> {
        if let Ok(output) = Command::new("fc-match")
            .args(["--format=%{file}", "DejaVuSans"])
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && Path::new(&path).is_file() {
                    return Some(PathBuf::from(path));
                }
            }
        }
        for candidate in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/run/current-system/sw/share/fonts/truetype/DejaVuSans.ttf",
            "/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf",
        ] {
            if Path::new(candidate).is_file() {
                return Some(PathBuf::from(candidate));
            }
        }
        None
    }

    macro_rules! need_font {
        () => {
            match find_regular_font() {
                Some(path) => path,
                None => {
                    eprintln!("skipping: no DejaVuSans-like TrueType font found on this system");
                    return;
                }
            }
        };
    }

    #[test]
    fn discover_returns_none_with_nothing_configured() {
        let flags = FontFlags::default();
        assert!(FontRegistry::discover(None, None, &flags)
            .unwrap()
            .is_none());
    }

    #[test]
    fn discover_returns_none_when_root_has_no_hash_dir() {
        let dir = tmpdir("no-hash-dir");
        let flags = FontFlags::default();
        assert!(
            FontRegistry::discover(Some(&dir), None, &flags)
                .unwrap()
                .is_none(),
            "an existing root with no dist/hash/fonts.satysfi-hash is 'nothing configured'"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_parses_single_and_collection_sources() {
        let dir = tmpdir("parse-sources");
        write_hash_dir(
            &dir,
            r#"{ "lmroman": { "src": "dist/fonts/lmroman.otf" },
                 "somettc": { "src": "dist/fonts/foo.ttc", "index": 2 } }"#,
            Some(r#"{ "regular": "lmroman" }"#),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .expect("config present");
        assert_eq!(
            registry.faces().get("lmroman"),
            Some(&FontSource::Single(dir.join("dist/fonts/lmroman.otf")))
        );
        assert_eq!(
            registry.faces().get("somettc"),
            Some(&FontSource::Collection(dir.join("dist/fonts/foo.ttc"), 2))
        );
        // regular/bold/oblique all fall back to the sole configured abbrev.
        assert_eq!(registry.default_faces, ["lmroman", "lmroman", "lmroman"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_resolves_absolute_src_verbatim() {
        let dir = tmpdir("abs-src");
        // An absolute path elsewhere on disk (need not exist: discover never
        // reads font file bytes, only the two config files).
        let abs = dir.join("elsewhere/regular.ttf");
        write_hash_dir(
            &dir,
            &format!(r#"{{ "abbr": {{ "src": {:?} }} }}"#, abs.to_str().unwrap()),
            Some(r#"{ "regular": "abbr" }"#),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .unwrap();
        assert_eq!(registry.faces().get("abbr"), Some(&FontSource::Single(abs)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_font_dir_takes_precedence_over_lib_root() {
        let lib_root = tmpdir("precedence-lib-root");
        let font_root = tmpdir("precedence-font-root");
        write_hash_dir(
            &lib_root,
            r#"{ "fromlib": { "src": "a.ttf" } }"#,
            Some(r#"{ "regular": "fromlib" }"#),
        );
        write_hash_dir(
            &font_root,
            r#"{ "fromfontdir": { "src": "b.ttf" } }"#,
            Some(r#"{ "regular": "fromfontdir" }"#),
        );
        let registry =
            FontRegistry::discover(Some(&lib_root), Some(&font_root), &FontFlags::default())
                .unwrap()
                .unwrap();
        assert!(registry.faces().contains_key("fromfontdir"));
        assert!(!registry.faces().contains_key("fromlib"));
        std::fs::remove_dir_all(&lib_root).ok();
        std::fs::remove_dir_all(&font_root).ok();
    }

    #[test]
    fn default_face_bold_and_oblique_can_diverge_from_regular() {
        let dir = tmpdir("distinct-faces");
        write_hash_dir(
            &dir,
            r#"{ "reg": { "src": "reg.ttf" },
                 "b":   { "src": "b.ttf" },
                 "obl": { "src": "obl.ttf" } }"#,
            Some(r#"{ "regular": "reg", "bold": "b", "oblique": "obl" }"#),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .unwrap();
        assert_eq!(registry.default_faces, ["reg", "b", "obl"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_json_is_an_error_not_none() {
        let dir = tmpdir("malformed");
        write_hash_dir(&dir, "{ not json", None);
        let err = FontRegistry::discover(Some(&dir), None, &FontFlags::default()).unwrap_err();
        assert!(matches!(err, FontConfigError::Json { .. }), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_default_font_file_is_an_error_once_fonts_hash_exists() {
        let dir = tmpdir("missing-default");
        write_hash_dir(&dir, r#"{ "reg": { "src": "reg.ttf" } }"#, None);
        let err = FontRegistry::discover(Some(&dir), None, &FontFlags::default()).unwrap_err();
        assert!(matches!(err, FontConfigError::Io { .. }), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_default_abbrev_is_an_error() {
        let dir = tmpdir("unknown-abbrev");
        write_hash_dir(
            &dir,
            r#"{ "reg": { "src": "reg.ttf" } }"#,
            Some(r#"{ "regular": "does-not-exist" }"#),
        );
        let err = FontRegistry::discover(Some(&dir), None, &FontFlags::default()).unwrap_err();
        assert!(
            matches!(err, FontConfigError::UnknownAbbrev { .. }),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn from_flags_without_regular_is_an_error() {
        let flags = FontFlags {
            regular: None,
            bold: Some(PathBuf::from("bold.ttf")),
            oblique: None,
        };
        let err = FontRegistry::discover(None, None, &flags).unwrap_err();
        assert!(matches!(err, FontConfigError::RegularRequired), "{err}");
    }

    #[test]
    fn flags_take_precedence_over_any_directory() {
        let dir = tmpdir("flags-precedence");
        write_hash_dir(
            &dir,
            r#"{ "fromconfig": { "src": "a.ttf" } }"#,
            Some(r#"{ "regular": "fromconfig" }"#),
        );
        let flags = FontFlags {
            regular: Some(PathBuf::from("cli-regular.ttf")),
            bold: None,
            oblique: None,
        };
        let registry = FontRegistry::discover(Some(&dir), None, &flags)
            .unwrap()
            .unwrap();
        assert!(!registry.faces().contains_key("fromconfig"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_store_rejects_nonzero_collection_index_without_touching_disk() {
        // `resolve` short-circuits on the index check before `TtfFontStore`
        // ever tries to read a file, so bogus (non-existent) paths are fine
        // here — this test intentionally does not need a real font.
        let dir = tmpdir("bad-index");
        write_hash_dir(
            &dir,
            r#"{ "reg": { "src": "reg.ttf" },
                 "obl": { "src": "obl.ttc", "index": 1 } }"#,
            Some(r#"{ "regular": "reg", "oblique": "obl" }"#),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .unwrap();
        // `TtfFontStore` (the `Ok` side) is not `Debug`, so match directly
        // rather than `unwrap_err()` (which requires `T: Debug`).
        let Err(err) = registry.build_store() else {
            panic!("expected an UnsupportedCollectionIndex error");
        };
        assert!(
            matches!(
                err,
                FontConfigError::UnsupportedCollectionIndex { index: 1, .. }
            ),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_store_loads_a_real_font_end_to_end() {
        let font_path = need_font!();
        let dir = tmpdir("real-font");
        write_hash_dir(
            &dir,
            &format!(
                r#"{{ "reg": {{ "src": {:?} }} }}"#,
                font_path.to_str().unwrap()
            ),
            Some(r#"{ "regular": "reg" }"#),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .unwrap();
        let store = registry.build_store().expect("build_store should succeed");
        assert_eq!(store.num_files(), 1);
        let size = satysfi_backend::Length::pt(12.0);
        assert!(store
            .advance(satysfi_backend::FontKey(0), 'A', size)
            .is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_store_from_cli_flags_end_to_end() {
        let font_path = need_font!();
        let flags = FontFlags {
            regular: Some(font_path),
            bold: None,
            oblique: None,
        };
        let registry = FontRegistry::discover(None, None, &flags).unwrap().unwrap();
        let store = registry.build_store().expect("build_store should succeed");
        assert_eq!(store.num_files(), 1);
        // bold/oblique fall back to the regular slot (same file, no
        // duplicate load) exactly like a bare `TtfFontStore::load(p, None, None)`.
        let size = satysfi_backend::Length::pt(12.0);
        assert_eq!(
            store.advance(satysfi_backend::FontKey(0), 'A', size),
            store.advance(satysfi_backend::FontKey(1), 'A', size)
        );
    }
}
