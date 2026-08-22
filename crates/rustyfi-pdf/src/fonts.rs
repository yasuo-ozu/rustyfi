//! Font configuration discovery and resolution (Slice 1): reads a small,
//! plain-JSON font configuration and turns it into a [`TtfFontStore`] via
//! its existing [`TtfFontStore::load`], so a document naming a real font
//! renders through [`crate::render_pdf_ttf`] instead of falling back to
//! the base-14 metrics.
//!
//! This module only *resolves configuration into a store*; it does not
//! change how typesetting reaches fonts — `set-font-key` already routes a
//! `FontKey` through whatever `&dyn FontMetrics` the caller passed to
//! `compile_document_cst`.
//!
//! # Compatibility with SATySFi's own `fonts.satysfi-hash`
//!
//! SATySFi keys font selection through two files under
//! `<runtime>/dist/hash/`: `fonts.satysfi-hash` maps a font *abbrev* to a
//! font file, and `default-font.satysfi-hash` maps a *script* to `{
//! font-name = abbrev; ratio; rising }`. The filenames are upstream's, and so
//! is the directory layout — a font package installs into this tree, so
//! reading anything else would mean packages install faces no document can
//! name.
//!
//! `default-font` is plain JSON. `fonts` is Yojson: each entry is wrapped in
//! a variant that no JSON parser accepts, which [`yojson_to_json`] strips
//! (see its doc comment) before the schema below parses with `serde_json`.
//! Both path spellings are accepted: `src` and upstream's `src-dist`, which
//! resolve against different bases — see `RawFontEntry`'s field docs.
//!
//! `serde`/`serde_json` were chosen over a hand-rolled parser: once the
//! variant wrappers are gone the schema below is a plain, statically-shaped
//! JSON object — exactly what `#[derive(Deserialize)]` is for — and `serde`
//! is already a workspace dependency.
//!
//! # `fonts.satysfi-hash`
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
//! (with `ratio`/`rising` scaling knobs). This port defines a smaller,
//! unrelated schema at the same filename, seeding the three faces the
//! base-14 provider already has (`FontKey(0/1/2)` = regular/bold/oblique,
//! `base14.rs`), with the per-script scheme as an optional `scripts` block
//! (D1a, see `RawScripts`):
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
//! An optional `"math"` key (Slice B) names the abbrev `get-initial-context`
//! seeds `Context::math_font` with, e.g. `{ "regular": "Junicode", "math":
//! "lmmath" }` — `download-fonts.sh` wires this to the bundled
//! Latin Modern Math (`lmmath`, upstream SATySFi's own default math font),
//! falling back to `dejavu-math` only if LM Math is unavailable. Absent ⇒ no
//! math default is configured, and `math_font` stays at `Context::initial`'s
//! own seed (`FontKey(0)`, the regular text face).
//!
//! # Discovery and error handling
//!
//! See [`FontRegistry::discover`] for the full precedence chain. In short: a
//! missing configuration is "nothing configured" and resolves to `Ok(None)`,
//! so a caller with no fonts set up anywhere keeps the base-14 path. Once
//! *something* is found, though, further problems (malformed JSON, a
//! default-face abbrev that isn't defined, a font file that fails to load)
//! are real errors (`Err`) rather than a silent fall-back to base-14 —
//! deliberately, so a broken font configuration is never confused with "no
//! font configuration".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use rustyfi_backend::FontKey;

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
/// The full abbrev map is kept (not just the three resolved paths) so
/// `set-font` can resolve an *arbitrary* abbrev to a `FontKey` beyond the
/// three seeded slots.
#[derive(Debug, Clone)]
pub struct FontRegistry {
    faces: BTreeMap<String, FontSource>,
    /// `[regular, bold, oblique]` abbrevs. `bold`/`oblique` equal `regular`
    /// verbatim when the config left them unset, which `build_store` uses
    /// to decide when to pass `None` (rather than resolving and loading the
    /// same file a second time) to `TtfFontStore::load`.
    default_faces: [String; 3],
    /// Per-script default `(abbrev, ratio, rising)`, indexed by
    /// `Script`'s discriminant (D1a) — from `default-font.satysfi-hash`'s
    /// optional `scripts` block. `None` per-slot when that script wasn't
    /// named (or the whole block is absent, or the registry came from
    /// `--font`/CLI flags, which have no `scripts` concept at all).
    script_fonts: [Option<(String, f64, f64)>; 4],
    /// The abbrev named by `default-font.satysfi-hash`'s optional `"math"`
    /// key (Slice B) — the font `get-initial-context` seeds
    /// `Context::math_font` with. `None` when absent (or the registry came
    /// from `--font`/CLI flags, which have no `"math"` concept).
    math_font: Option<String>,
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

/// Rewrite Yojson variant syntax into the JSON `serde_json` accepts.
///
/// SATySFi's own `fonts.satysfi-hash` wraps each entry in a variant —
/// `<Single: {"src": "…"}>` as upstream writes it, `<"Collection":{"src-dist":
/// "…","index":1}>` as a package installer does — which no JSON parser takes.
/// The tag carries no information the fields do not (`index` is what
/// distinguishes a collection member), so the wrapper is simply removed,
/// leaving the object behind.
///
/// Anything inside a string literal is left alone: a font path may legitimately
/// contain `<` or `>`.
fn yojson_to_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut in_string = false;
    let mut escaped = false;
    let mut depth: usize = 0;

    while let Some((i, c)) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '<' => {
                // `<Tag:` or `<"Tag":` — skip up to and including the colon,
                // and remember to drop the matching `>`.
                let rest = &text[i + 1..];
                match rest.find(':') {
                    Some(colon)
                        if rest[..colon]
                            .chars()
                            .all(|t| t.is_alphanumeric() || t == '"' || t == '_' || t == '-' || t.is_whitespace()) =>
                    {
                        for _ in 0..=colon {
                            chars.next();
                        }
                        depth += 1;
                    }
                    // Not a variant tag; keep the character as it stands.
                    _ => out.push(c),
                }
            }
            '>' if depth > 0 => depth -= 1,
            _ => out.push(c),
        }
    }
    out
}

/// Raw shape of one entry in `fonts.satysfi-hash` (see module docs). `index`
/// present ⇒ a `.ttc` member.
#[derive(Debug, Deserialize)]
struct RawFontEntry {
    /// A path relative to the FONT ROOT (or absolute) — this port's spelling,
    /// and the one upstream files written by hand tend to use.
    #[serde(default)]
    src: Option<PathBuf>,
    /// Upstream's other spelling: relative to `dist/fonts/`, which is where a
    /// font package installs its faces. `(font "X.otf" …)` in a `Satyristes`
    /// lands at `dist/fonts/<package>/X.otf`, and the hash file names it
    /// `<package>/X.otf` — so this base is `dist/fonts`, not `dist`.
    #[serde(default, rename = "src-dist")]
    src_dist: Option<PathBuf>,
    #[serde(default)]
    index: Option<u32>,
}

impl RawFontEntry {
    /// The font file, resolved against `root`. `Path::join` discards `root`
    /// when the value is absolute, so both cases fall out of one call.
    fn resolve(&self, root: &std::path::Path) -> Option<PathBuf> {
        if let Some(src) = &self.src {
            return Some(root.join(src));
        }
        self.src_dist
            .as_ref()
            .map(|rel| root.join("dist").join("fonts").join(rel))
    }
}

/// Raw shape of `default-font.satysfi-hash` (port-specific; see module
/// docs). Only `regular` is required. `scripts` (D1a) is the optional
/// per-script default scheme mirroring upstream `setDefaultFont.ml`'s
/// shape — absent entirely ⇒ every script defaults to `(FontKey(0), 1.0,
/// 0.0)`, i.e. today's single-font behavior
/// (`TtfFontStore::script_default` returns `None`).
#[derive(Debug, Deserialize)]
struct RawDefaultFace {
    regular: String,
    #[serde(default)]
    bold: Option<String>,
    #[serde(default)]
    oblique: Option<String>,
    #[serde(default)]
    scripts: Option<RawScripts>,
    /// Slice B: the abbrev `get-initial-context` seeds
    /// `Context::math_font` with. Optional — absent means no math default is
    /// configured, and `Context::math_font` stays at `Context::initial`'s
    /// `FontKey(0)` seed.
    #[serde(default)]
    math: Option<String>,
}

/// One entry of the `scripts` block: `{ "font-name": abbrev, "ratio": f64,
/// "rising": f64 }`.
#[derive(Debug, Deserialize)]
struct RawScriptFont {
    #[serde(rename = "font-name")]
    font_name: String,
    ratio: f64,
    rising: f64,
}

/// The four script slots `default-font.satysfi-hash`'s `scripts` block may
/// name, each optional (an absent script keeps the `(FontKey(0), 1.0, 0.0)`
/// default). Field names mirror upstream's own script identifiers
/// (`han-ideographic`/`kana`/`latin`/`other-script`).
#[derive(Debug, Deserialize, Default)]
struct RawScripts {
    #[serde(rename = "han-ideographic")]
    han_ideographic: Option<RawScriptFont>,
    kana: Option<RawScriptFont>,
    latin: Option<RawScriptFont>,
    #[serde(rename = "other-script")]
    other_script: Option<RawScriptFont>,
}

/// Load `path`'s bytes into `files` (via
/// [`TtfFontStore::read_and_validate`]), or reuse an already-loaded file's
/// index when `path` canonicalizes to one already in `file_by_path` (D1a
/// dedup: two abbrevs naming the same physical font file share one embedded
/// copy). Falls back to the path as-given when canonicalization fails (a
/// bad path is then reported by `read_and_validate`'s own `Io` error,
/// rather than silently treated as "never seen before" and read a second
/// time under a slightly different string).
fn load_or_dedup(
    files: &mut Vec<Vec<u8>>,
    file_by_path: &mut BTreeMap<PathBuf, usize>,
    path: &Path,
) -> Result<usize, FontConfigError> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(&idx) = file_by_path.get(&canon) {
        return Ok(idx);
    }
    let bytes = TtfFontStore::read_and_validate(path)?;
    let idx = files.len();
    files.push(bytes);
    file_by_path.insert(canon, idx);
    Ok(idx)
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
    ///    `--font-dir` flag and `$RUSTYFI_FONT_DIR` into this one
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
        // Upstream writes Yojson, so the bytes may not be JSON yet.
        let fonts_text = String::from_utf8_lossy(&fonts_bytes);
        let raw: BTreeMap<String, RawFontEntry> =
            serde_json::from_str(&yojson_to_json(&fonts_text)).map_err(|source| {
                FontConfigError::Json {
                    path: fonts_path.clone(),
                    source,
                }
            })?;
        let faces: BTreeMap<String, FontSource> = raw
            .into_iter()
            .filter_map(|(abbrev, entry)| {
                // An entry naming no file at all is skipped rather than
                // resolved to the root directory itself.
                let resolved = entry.resolve(root)?;
                let source = match entry.index {
                    Some(index) => FontSource::Collection(resolved, index),
                    None => FontSource::Single(resolved),
                };
                Some((abbrev, source))
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

        // D1a: validate + resolve the optional `scripts` block. Same
        // doctrine as the three default faces above — a script naming an
        // abbrev absent from `faces` is a broken config (`Err`), never a
        // silent fall-back.
        let mut script_fonts: [Option<(String, f64, f64)>; 4] = [None, None, None, None];
        if let Some(scripts) = raw_default.scripts {
            for (idx, name, entry) in [
                (0usize, "han-ideographic", scripts.han_ideographic),
                (1, "kana", scripts.kana),
                (2, "latin", scripts.latin),
                (3, "other-script", scripts.other_script),
            ] {
                let Some(entry) = entry else { continue };
                if !faces.contains_key(&entry.font_name) {
                    return Err(FontConfigError::UnknownAbbrev {
                        path: default_path,
                        face: name,
                        abbrev: entry.font_name,
                    });
                }
                script_fonts[idx] = Some((entry.font_name, entry.ratio, entry.rising));
            }
        }

        // Slice B: same doctrine for the optional `"math"` abbrev — a config
        // that NAMES a math default but gets the abbrev wrong is a broken
        // config (`Err`), never a silent "no math default configured".
        if let Some(abbrev) = &raw_default.math {
            if !faces.contains_key(abbrev) {
                return Err(FontConfigError::UnknownAbbrev {
                    path: default_path,
                    face: "math",
                    abbrev: abbrev.clone(),
                });
            }
        }

        Ok(Some(FontRegistry {
            faces,
            default_faces: [regular, bold, oblique],
            script_fonts,
            math_font: raw_default.math,
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
            script_fonts: [None, None, None, None],
            math_font: None,
        })
    }

    /// Resolve every configured abbrev and build an N-slot [`TtfFontStore`]
    /// (D1a): the three seeded default faces occupy `FontKey(0/1/2)`, and
    /// every OTHER abbrev in [`Self::faces`] gets its own slot beyond that,
    /// deduped against every file already loaded (by canonical path) so two
    /// abbrevs naming the same physical font file share one embedded copy.
    ///
    /// **Eager, not lazy.** Upstream (`fontInfo.ml:24-132`) loads a face the
    /// first time a script/abbrev actually needs it. `TtfFontStore` cannot
    /// do that without unsafe self-referential storage or a crate like
    /// `owned-ttf-parser` (see `ttf.rs`'s struct doc on why `Face` is
    /// reparsed on demand instead of cached) — over a realistic registry
    /// (~11 files, ~20 MB for the stdja family) eager loading is simpler and
    /// cheap enough; revisit only if a huge registry appears.
    pub fn build_store(&self) -> Result<TtfFontStore, FontConfigError> {
        // Resolve every path FIRST (can fail with `UnsupportedCollectionIndex`
        // — a pure config check, no I/O) before any file is actually read, so
        // a bad config abbrev is reported without touching disk at all
        // (`build_store_rejects_nonzero_collection_index_without_touching_
        // disk`).
        let regular_path = self.resolve(&self.default_faces[0])?;
        let bold_path = if self.default_faces[1] != self.default_faces[0] {
            Some(self.resolve(&self.default_faces[1])?)
        } else {
            None
        };
        let oblique_path = if self.default_faces[2] != self.default_faces[0] {
            Some(self.resolve(&self.default_faces[2])?)
        } else {
            None
        };
        let mut other_paths: Vec<(String, PathBuf)> = Vec::new();
        for abbrev in self.faces.keys() {
            if self.default_faces.contains(abbrev) {
                continue; // handled below, mapped to FontKey(0/1/2).
            }
            other_paths.push((abbrev.clone(), self.resolve(abbrev)?));
        }

        let mut files: Vec<Vec<u8>> = Vec::new();
        let mut file_by_path: BTreeMap<PathBuf, usize> = BTreeMap::new();

        // Step 1: the three default slots, in `FontKey(0/1/2)` order —
        // dedup only regular-vs-{bold,oblique} by ABBREV-NAME equality
        // (`TtfFontStore::load`'s own bold/oblique-falls-back-to-regular
        // convention).
        let regular_idx = load_or_dedup(&mut files, &mut file_by_path, &regular_path)?;
        let mut slots = vec![regular_idx, regular_idx, regular_idx];
        if let Some(path) = &bold_path {
            slots[1] = load_or_dedup(&mut files, &mut file_by_path, path)?;
        }
        if let Some(path) = &oblique_path {
            slots[2] = load_or_dedup(&mut files, &mut file_by_path, path)?;
        }

        // Step 2: every other configured abbrev gets its own slot
        // (`FontKey(3)`, `FontKey(4)`, ... in map-iteration/abbrev-sorted
        // order — the exact allocation order is not part of the contract,
        // only that each distinct abbrev gets a distinct `FontKey` unless
        // it shares a file with one already loaded).
        let mut abbrevs: BTreeMap<String, FontKey> = BTreeMap::new();
        for (abbrev, path) in &other_paths {
            let idx = load_or_dedup(&mut files, &mut file_by_path, path)?;
            let key = FontKey(slots.len() as u16);
            slots.push(idx);
            abbrevs.insert(abbrev.clone(), key);
        }

        // Step 3: the three default-face abbrevs resolve to FontKey(0/1/2)
        // regardless of what slot (if any) step 2 gave a same-named-but-
        // different-abbrev file — `resolve_font_abbrev("Junicode")` must
        // agree with `set-font-key 0` when "Junicode" IS the regular face.
        // `or_insert` (not a plain overwrite): when bold/oblique default to
        // the SAME abbrev string as regular (the common case — no bold/
        // oblique configured), all three loop iterations see that one
        // string, and the FIRST (smallest, i.e. regular's own FontKey(0))
        // must win, not the last.
        for (i, abbrev) in self.default_faces.iter().enumerate() {
            abbrevs.entry(abbrev.clone()).or_insert(FontKey(i as u16));
        }

        // `scripts` block: resolve each configured abbrev to the FontKey
        // just allocated for it (always present — `discover` validated
        // every `scripts` abbrev against `faces` up front).
        let mut script_defaults: [Option<(FontKey, f64, f64)>; 4] = [None, None, None, None];
        for (i, entry) in self.script_fonts.iter().enumerate() {
            if let Some((abbrev, ratio, rising)) = entry {
                let key = *abbrevs.get(abbrev).unwrap_or_else(|| {
                    panic!("FontRegistry invariant violated: scripts abbrev {abbrev:?} unresolved")
                });
                script_defaults[i] = Some((key, *ratio, *rising));
            }
        }

        // Slice B: resolve the configured `"math"` abbrev (if any) — same
        // lookup as the `scripts` block above.
        let math_default = self.math_font.as_ref().map(|abbrev| {
            *abbrevs.get(abbrev).unwrap_or_else(|| {
                panic!("FontRegistry invariant violated: math abbrev {abbrev:?} unresolved")
            })
        });

        Ok(TtfFontStore::from_parts(
            files,
            slots,
            abbrevs,
            script_defaults,
            math_default,
        ))
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

    /// The abbrev -> font source map.
    pub fn faces(&self) -> &BTreeMap<String, FontSource> {
        &self.faces
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_backend::FontMetrics as _;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmpdir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "rustyfi-pdf-fonts-test-{tag}-{}-{}-{n}",
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
        let size = rustyfi_backend::Length::pt(12.0);
        assert!(store
            .advance(rustyfi_backend::FontKey(0), 'A', size)
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
        let size = rustyfi_backend::Length::pt(12.0);
        assert_eq!(
            store.advance(rustyfi_backend::FontKey(0), 'A', size),
            store.advance(rustyfi_backend::FontKey(1), 'A', size)
        );
    }

    // ---- D1a: N-slot store, abbrev roundtrip, file dedup, `scripts` block
    // --------------------------------

    /// A second real TrueType file, distinct from `find_regular_font`'s
    /// (DejaVu Sans Mono vs. DejaVu Sans) — needed to prove a genuinely
    /// different abbrev gets its own physical-file slot, not just dedup.
    fn find_second_font() -> Option<PathBuf> {
        if let Ok(output) = Command::new("fc-match")
            .args(["--format=%{file}", "DejaVu Sans Mono"])
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
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/run/current-system/sw/share/fonts/truetype/DejaVuSansMono.ttf",
        ] {
            if Path::new(candidate).is_file() {
                return Some(PathBuf::from(candidate));
            }
        }
        None
    }

    macro_rules! need_second_font {
        () => {
            match find_second_font() {
                Some(path) => path,
                None => {
                    eprintln!("skipping: no DejaVuSansMono-like TrueType font found");
                    return;
                }
            }
        };
    }

    #[test]
    fn build_store_allocates_extra_slots_and_dedups_shared_files() {
        let regular = need_font!();
        let mono = need_second_font!();
        let dir = tmpdir("extra-abbrevs");
        write_hash_dir(
            &dir,
            &format!(
                r#"{{ "reg": {{ "src": {:?} }},
                     "mono": {{ "src": {:?} }},
                     "regalias": {{ "src": {:?} }} }}"#,
                regular.to_str().unwrap(),
                mono.to_str().unwrap(),
                regular.to_str().unwrap(),
            ),
            Some(r#"{ "regular": "reg" }"#),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .unwrap();
        let store = registry.build_store().expect("build_store should succeed");

        // Two distinct physical files ("reg"/"regalias" share one; "mono" is
        // its own), five allocated FontKey slots (0/1/2 default + mono +
        // regalias).
        assert_eq!(store.num_files(), 2, "reg and regalias must dedup to one file");
        assert_eq!(store.num_slots(), 5);

        // abbrev_key roundtrip: the three default-face abbrevs resolve to
        // FontKey(0/1/2) regardless of iteration order (step 3 override);
        // the two "extra" abbrevs get later slots.
        assert_eq!(store.abbrev_key("reg"), Some(rustyfi_backend::FontKey(0)));
        let mono_key = store.abbrev_key("mono").expect("mono abbrev resolves");
        let regalias_key = store.abbrev_key("regalias").expect("regalias abbrev resolves");
        assert_ne!(mono_key, rustyfi_backend::FontKey(0));
        assert_ne!(regalias_key, rustyfi_backend::FontKey(0));
        assert_ne!(mono_key, regalias_key);
        assert_eq!(store.abbrev_key("no-such-abbrev"), None);

        // File-index dedup: "regalias" backs the SAME physical file as the
        // regular slot; "mono" backs a DIFFERENT one.
        assert_eq!(store.file_index(regalias_key), store.file_index(rustyfi_backend::FontKey(0)));
        assert_ne!(store.file_index(mono_key), store.file_index(rustyfi_backend::FontKey(0)));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scripts_block_parses_and_resolves_default_script_font() {
        let regular = need_font!();
        let cjk_stand_in = need_second_font!();
        let dir = tmpdir("scripts-block");
        write_hash_dir(
            &dir,
            &format!(
                r#"{{ "reg": {{ "src": {:?} }}, "cjk": {{ "src": {:?} }} }}"#,
                regular.to_str().unwrap(),
                cjk_stand_in.to_str().unwrap(),
            ),
            Some(
                r#"{ "regular": "reg",
                     "scripts": {
                       "han-ideographic": { "font-name": "cjk", "ratio": 0.88, "rising": 0.0 },
                       "latin":           { "font-name": "reg", "ratio": 1.0,  "rising": 0.0 }
                     } }"#,
            ),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .unwrap();
        let store = registry.build_store().expect("build_store should succeed");

        let cjk_key = store.abbrev_key("cjk").expect("cjk abbrev resolves");
        // Script indices: HanIdeographic=0, Kana=1, Latin=2, OtherScript=3
        // (`context::Script`'s discriminants).
        assert_eq!(store.script_default(0), Some((cjk_key, 0.88, 0.0)));
        assert_eq!(
            store.script_default(2),
            Some((rustyfi_backend::FontKey(0), 1.0, 0.0))
        );
        // Unconfigured scripts stay `None` (caller falls back to
        // `(ctx.font, 1.0, 0.0)`).
        assert_eq!(store.script_default(1), None); // Kana
        assert_eq!(store.script_default(3), None); // OtherScript

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scripts_block_is_absent_by_default() {
        let font_path = need_font!();
        let dir = tmpdir("no-scripts-block");
        write_hash_dir(
            &dir,
            &format!(r#"{{ "reg": {{ "src": {:?} }} }}"#, font_path.to_str().unwrap()),
            Some(r#"{ "regular": "reg" }"#),
        );
        let registry = FontRegistry::discover(Some(&dir), None, &FontFlags::default())
            .unwrap()
            .unwrap();
        let store = registry.build_store().expect("build_store should succeed");
        for script in 0..4 {
            assert_eq!(store.script_default(script), None);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scripts_block_unknown_abbrev_is_an_error() {
        let font_path = need_font!();
        let dir = tmpdir("scripts-unknown-abbrev");
        write_hash_dir(
            &dir,
            &format!(r#"{{ "reg": {{ "src": {:?} }} }}"#, font_path.to_str().unwrap()),
            Some(
                r#"{ "regular": "reg",
                     "scripts": { "kana": { "font-name": "nope", "ratio": 1.0, "rising": 0.0 } } }"#,
            ),
        );
        let err = FontRegistry::discover(Some(&dir), None, &FontFlags::default()).unwrap_err();
        assert!(matches!(err, FontConfigError::UnknownAbbrev { .. }), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod satysfi_compat_tests {
    use super::*;

    /// Both spellings seen in the wild: upstream's own unquoted tag with a
    /// root-relative `src`, and a package installer's quoted tag with a
    /// `dist/`-relative `src-dist`.
    const UPSTREAM: &str = r#"{
  "fonts-noto-emoji:NotoEmoji-Regular" : <Single: {"src": "dist/fonts/fonts-noto-emoji/NotoEmoji-Regular.ttf"}>,
  "fonts-theano:TheanoDidot":<"Single":{"src-dist":"fonts-theano/TheanoDidot-Regular.otf"}>,
  "somettc":<"Collection":{"src-dist":"x/foo.ttc","index":0}>
}"#;

    #[test]
    fn upstream_variants_become_plain_json() {
        let json = yojson_to_json(UPSTREAM);
        assert!(!json.contains('<') && !json.contains('>'), "{json}");
        let raw: BTreeMap<String, RawFontEntry> =
            serde_json::from_str(&json).expect("should parse once the variants are gone");
        assert_eq!(raw.len(), 3);
        assert_eq!(
            raw["fonts-theano:TheanoDidot"].src_dist.as_deref(),
            Some(std::path::Path::new("fonts-theano/TheanoDidot-Regular.otf"))
        );
        assert_eq!(raw["somettc"].index, Some(0));
    }

    #[test]
    fn src_and_src_dist_resolve_from_different_bases() {
        let root = std::path::Path::new("/root");
        let by_src = RawFontEntry {
            src: Some("dist/fonts/a.ttf".into()),
            src_dist: None,
            index: None,
        };
        let by_dist = RawFontEntry {
            src: None,
            src_dist: Some("fonts-theano/b.otf".into()),
            index: None,
        };
        assert_eq!(by_src.resolve(root).unwrap(), root.join("dist/fonts/a.ttf"));
        assert_eq!(
            by_dist.resolve(root).unwrap(),
            root.join("dist/fonts/fonts-theano/b.otf"),
            "`src-dist` is relative to dist/fonts/ — where a package installs"
        );
    }

    #[test]
    fn an_angle_bracket_inside_a_string_survives() {
        // A path may contain `<`; only variant wrappers are removed.
        let json = yojson_to_json(r#"{"a":<Single: {"src":"we<ird>.ttf"}>}"#);
        assert_eq!(json, r#"{"a": {"src":"we<ird>.ttf"}}"#);
    }
}
