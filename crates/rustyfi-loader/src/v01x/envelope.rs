//! `rustyfi-envelope.yaml` decoder — Ld3b-1 (Axis B increment Ld3b, §3.2/§5.2
//! of `/home/yasuo/.claude/jobs/a7244c0b/tmp/axis-b-ld3b.md`). Transcribed
//! from `saphe-split @ b836d512689248d18970674021ecaca409e0d897`,
//! `src/frontend/envelopeConfig.ml` (decoder) +
//! `src-common/envelopeSystemBase.ml:11-87` (record shapes) +
//! `src-common/commonUtil.ml:12-40` (field validators, `parse_long_command`
//! / `parse_long_identifier`) + `src/frontend/configUtil.ml`
//! (`relative_path_decoder`, `lowercased_identifier_decoder`).
//!
//! This is the compiler-side **decoder only**: [`load_config`] decodes and
//! validates one `rustyfi-envelope.yaml` file. The Ld3b-2 "reader" half
//! (`envelopeReader.ml`'s directory listing + per-source parse, i.e.
//! `EnvelopeSource`/`ReadEnvelope`/`read`) is NOT built here — nothing in
//! this crate calls [`load_config`] yet. `#![allow(dead_code)]` reflects
//! exactly that; every item here is exercised by this module's own unit
//! tests, including all 19 real upstream `rustyfi-envelope.yaml.expected`
//! fixtures (3 committed verbatim under
//! `tests/fixtures/v01x/envelope/`, all 19 via the env-gated sweep test
//! below).
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use rustyfi_syntax::cst_v1::FileV1;

use crate::error::LoadError;

/// Candidate source-file extensions, filtered by SUFFIX match — upstream
/// `envelopeReader.ml:20` (`Core.String.is_suffix`), the same `[".satyh";
/// ".satyg"]` list `get_candidate_file_extensions PdfMode` threads into both
/// resolvers (`main.ml:90-93`); same as `open_doc::CANDIDATE_EXTS`.
const SOURCE_EXTS: [&str; 2] = [".satyh", ".satyg"];

/// Decoded `rustyfi-envelope.yaml` — `envelopeConfig.ml:118-139` /
/// `envelopeSystemBase.ml:71-87`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvelopeConfig {
    pub contents: EnvelopeContents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnvelopeContents {
    Library {
        /// Plain string — NOT identifier-validated (`envelopeConfig.ml:122`
        /// uses `string`, not `uppercased_identifier_decoder`).
        main_module: String,
        /// Relative directory strings, joined to the config file's
        /// directory at read time (Ld3b-2). NOT validated as relative here
        /// — upstream's own decoder is plain `list string`
        /// (`envelopeConfig.ml:123`), unlike `font_file_description.path`
        /// which *is* `relative_path_decoder`.
        source_directories: Vec<String>,
        test_directories: Vec<String>,
        markdown_conversion: Option<MarkdownConversion>,
    },
    Font {
        main_module: String,
        files: Vec<FontFileDescription>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FontFileDescription {
    /// Relative path (empty allowed) — `configUtil.ml:36-42`
    /// (`relative_path_decoder`), `envelopeConfig.ml:35`.
    pub path: String,
    pub contents: FontFileContents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FontFileContents {
    OpentypeSingle(FontSpec),
    OpentypeCollection(Vec<FontSpec>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FontSpec {
    /// Lowercased identifier (`configUtil.ml:19-25`).
    pub name: String,
    pub math: bool,
}

/// All 19 fields of `envelopeSystemBase.ml:46-69`, held as VALIDATED but
/// UNINTERPRETED command names — nothing consumes them yet (the markdown
/// input kind is out of scope, Ld3b spec §12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkdownConversion {
    pub document: LongIdentifier,
    pub paragraph: LongCommand,
    pub hr: LongCommand,
    pub h1: LongCommand,
    pub h2: LongCommand,
    pub h3: LongCommand,
    pub h4: LongCommand,
    pub h5: LongCommand,
    pub h6: LongCommand,
    pub ul: LongCommand,
    pub ol: LongCommand,
    pub code_block: LongCommand,
    pub blockquote: LongCommand,
    pub emph: LongCommand,
    pub strong: LongCommand,
    /// `get_opt` — absent OR explicit YAML `null` both decode to `None`
    /// (`yamlDecoder.ml:101-111`; the `md-ja` fixture's `hard_break:` with
    /// no value is the null case).
    pub hard_break: Option<LongCommand>,
    pub code: LongCommand,
    pub link: LongCommand,
    pub img: LongCommand,
}

/// A `+`- or `\`-prefixed dotted command name, split into its module chain
/// and final (lowercased) component — `long_block_command`/
/// `long_inline_command`, `envelopeSystemBase.ml:29-39`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LongCommand {
    pub modules: Vec<String>,
    pub name: String,
}

/// A dotted identifier (no command prefix) — `long_identifier`,
/// `envelopeSystemBase.ml:41-44`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LongIdentifier {
    pub modules: Vec<String>,
    pub name: String,
}

// -- raw serde layer (private) --
//
// The `library`/`font` branch (and the `opentype_single`/
// `opentype_collection` branch inside `font_file_description`) is decoded
// as a plain struct of `Option`s + an explicit exactly-one check, NOT a
// serde externally-tagged `enum`: upstream's `branch` combinator
// (`yamlDecoder.ml:161-193`) tolerates extra non-tag keys beside the tag
// key and gives named errors for 0-hit / 2-hit, which an externally-tagged
// enum's single-key-map encoding cannot express (Ld3b spec §3.2).

#[derive(serde::Deserialize)]
struct EnvelopeConfigRaw {
    #[serde(default)]
    library: Option<LibraryRaw>,
    #[serde(default)]
    font: Option<FontRaw>,
}

#[derive(serde::Deserialize)]
struct LibraryRaw {
    main_module: String,
    source_directories: Vec<String>,
    test_directories: Vec<String>,
    #[serde(default)]
    markdown_conversion: Option<MarkdownConversionRaw>,
}

#[derive(serde::Deserialize)]
struct FontRaw {
    main_module: String,
    files: Vec<FontFileDescriptionRaw>,
}

#[derive(serde::Deserialize)]
struct FontFileDescriptionRaw {
    path: String,
    #[serde(default)]
    opentype_single: Option<FontSpecRaw>,
    #[serde(default)]
    opentype_collection: Option<Vec<FontSpecRaw>>,
}

#[derive(serde::Deserialize)]
struct FontSpecRaw {
    name: String,
    math: bool,
}

#[derive(serde::Deserialize)]
struct MarkdownConversionRaw {
    document: String,
    paragraph: String,
    hr: String,
    h1: String,
    h2: String,
    h3: String,
    h4: String,
    h5: String,
    h6: String,
    ul: String,
    ol: String,
    code_block: String,
    blockquote: String,
    emph: String,
    strong: String,
    /// `#[serde(default)]` makes the *key itself* optional (upstream's
    /// `get_opt`); `Option<String>` additionally absorbs an explicit YAML
    /// `null` (upstream's `option`, `yamlDecoder.ml:101-106`) since serde's
    /// `Option<T>` deserializes `null` as `None` for any present key.
    #[serde(default)]
    hard_break: Option<String>,
    code: String,
    link: String,
    img: String,
}

/// Decode + validate one `rustyfi-envelope.yaml`. ≈ `EnvelopeConfig.load`,
/// `envelopeConfig.ml:142-149`.
pub(crate) fn load_config(path: &Path) -> Result<EnvelopeConfig, LoadError> {
    let text =
        std::fs::read_to_string(path).map_err(|source| LoadError::EnvelopeConfigNotFound {
            path: path.to_path_buf(),
            source,
        })?;
    decode(&text).map_err(|message| LoadError::EnvelopeConfigDecode {
        path: path.to_path_buf(),
        message,
    })
}

fn decode(text: &str) -> Result<EnvelopeConfig, String> {
    let raw: EnvelopeConfigRaw = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    convert_config(raw)
}

/// One source file of a read envelope — Ld3b-2. Its `file` is always a
/// [`FileV1::Library`] (a document among the sources is a hard error,
/// upstream `NotALibraryFile`), and `module_name` is its declared module
/// name (`FileV1::Library.name.name`).
#[derive(Debug)]
pub(crate) struct EnvelopeSource {
    /// Canonicalized path to the source file on disk.
    pub path: PathBuf,
    pub file: FileV1,
    /// The declared module name — the key the closed resolver graphs on.
    pub module_name: String,
}

/// A read envelope: its decoded config plus every parsed library source file
/// (empty for a `font:` envelope) — Ld3b-2.
#[derive(Debug)]
pub(crate) struct ReadEnvelope {
    pub config: EnvelopeConfig,
    pub sources: Vec<EnvelopeSource>,
}

/// Read an envelope — ≈ `EnvelopeReader.main`, `envelopeReader.ml:29-87`.
///
/// Decodes `config_path`, then (Library only) lists every
/// `source_directories` entry joined to the CONFIG FILE's directory
/// (`envelopeReader.ml:32,36`), one flat `readdir` each (no recursion,
/// `:15-16`), filtered by the `.satyh`/`.satyg` SUFFIX (`:20`) and SORTED for
/// determinism (upstream's `readdir` order is OS-arbitrary; sorting is
/// strictly more deterministic and unobservable after the closed sort). Each
/// file is parsed with `parse_file_v1`; a document among them is an error
/// (upstream `NotALibraryFile` → [`LoadError::DocumentAsDependency`]).
/// `test_directories` are ignored (`use_test_files: false` always — depended
/// envelopes never use their tests, `closedEnvelopeDependencyResolver.ml:45`;
/// `saphe test` is out of scope). A `font:` envelope yields `sources: []` —
/// NOT an error (upstream `UTFontEnvelope` parses nothing, `:71-85`).
pub(crate) fn read(config_path: &Path) -> Result<ReadEnvelope, LoadError> {
    let config = load_config(config_path)?;
    let dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let sources = match &config.contents {
        EnvelopeContents::Library {
            source_directories, ..
        } => {
            let mut source_paths: Vec<PathBuf> = Vec::new();
            for sd in source_directories {
                let absdir = dir.join(sd);
                source_paths.extend(list_sources_in_directory(&absdir)?);
            }
            let mut sources = Vec::with_capacity(source_paths.len());
            for path in source_paths {
                let src = std::fs::read_to_string(&path).map_err(|source| LoadError::Io {
                    path: path.clone(),
                    source,
                })?;
                let file = rustyfi_syntax::parse_file_v1(&src).map_err(|source| {
                    LoadError::Parse {
                        path: path.clone(),
                        source,
                    }
                })?;
                let module_name = match &file {
                    FileV1::Library { name, .. } => name.name.clone(),
                    // A document file among an envelope's sources is an error
                    // (upstream `NotALibraryFile`, `envelopeReader.ml:60-61`).
                    FileV1::Document { .. } => {
                        return Err(LoadError::DocumentAsDependency { path });
                    }
                };
                let path = crate::canonicalize(&path)?;
                sources.push(EnvelopeSource {
                    path,
                    file,
                    module_name,
                });
            }
            sources
        }
        // Font envelopes parse no sources (upstream returns `UTFontEnvelope`
        // without listing/parsing anything); they still occupy a graph
        // vertex, they just contribute zero files.
        EnvelopeContents::Font { .. } => Vec::new(),
    };

    Ok(ReadEnvelope { config, sources })
}

/// One flat `readdir` of `absdir`, filtered by the `.satyh`/`.satyg` suffix
/// and sorted — ≈ `listup_sources_in_directory`, `envelopeReader.ml:12-26`.
fn list_sources_in_directory(absdir: &Path) -> Result<Vec<PathBuf>, LoadError> {
    let read_dir = std::fs::read_dir(absdir).map_err(|source| LoadError::CannotReadDirectory {
        path: absdir.to_path_buf(),
        source,
    })?;
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|source| LoadError::CannotReadDirectory {
            path: absdir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if SOURCE_EXTS.iter().any(|ext| name.ends_with(ext)) {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

fn convert_config(raw: EnvelopeConfigRaw) -> Result<EnvelopeConfig, String> {
    let contents = match (raw.library, raw.font) {
        (Some(lib), None) => convert_library(lib)?,
        (None, Some(font)) => convert_font(font)?,
        (None, None) => {
            return Err("$: expected exactly one of: library, font (got none)".to_string());
        }
        (Some(_), Some(_)) => {
            return Err("$: expected exactly one of: library, font (got both)".to_string());
        }
    };
    Ok(EnvelopeConfig { contents })
}

fn convert_library(raw: LibraryRaw) -> Result<EnvelopeContents, String> {
    let markdown_conversion = raw
        .markdown_conversion
        .map(|m| convert_markdown_conversion(m, "library.markdown_conversion"))
        .transpose()?;
    Ok(EnvelopeContents::Library {
        main_module: raw.main_module,
        source_directories: raw.source_directories,
        test_directories: raw.test_directories,
        markdown_conversion,
    })
}

fn convert_font(raw: FontRaw) -> Result<EnvelopeContents, String> {
    let files = raw
        .files
        .into_iter()
        .enumerate()
        .map(|(i, f)| convert_font_file_description(f, &format!("font.files.[{i}]")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EnvelopeContents::Font {
        main_module: raw.main_module,
        files,
    })
}

fn convert_font_file_description(
    raw: FontFileDescriptionRaw,
    ctx: &str,
) -> Result<FontFileDescription, String> {
    if !is_relative_path(&raw.path) {
        return Err(format!("{ctx}.path: not a relative path: `{}`", raw.path));
    }
    let contents = match (raw.opentype_single, raw.opentype_collection) {
        (Some(single), None) => FontFileContents::OpentypeSingle(convert_font_spec(
            single,
            &format!("{ctx}.opentype_single"),
        )?),
        (None, Some(list)) => {
            let specs = list
                .into_iter()
                .enumerate()
                .map(|(i, s)| {
                    convert_font_spec(s, &format!("{ctx}.opentype_collection.[{i}]"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            FontFileContents::OpentypeCollection(specs)
        }
        (None, None) => {
            return Err(format!(
                "{ctx}: expected exactly one of: opentype_single, opentype_collection (got none)"
            ));
        }
        (Some(_), Some(_)) => {
            return Err(format!(
                "{ctx}: expected exactly one of: opentype_single, opentype_collection (got both)"
            ));
        }
    };
    Ok(FontFileDescription {
        path: raw.path,
        contents,
    })
}

fn convert_font_spec(raw: FontSpecRaw, ctx: &str) -> Result<FontSpec, String> {
    if !is_lowercased_identifier(&raw.name) {
        return Err(format!(
            "{ctx}.name: not a lowercased identifier: `{}`",
            raw.name
        ));
    }
    Ok(FontSpec {
        name: raw.name,
        math: raw.math,
    })
}

fn convert_markdown_conversion(
    raw: MarkdownConversionRaw,
    ctx: &str,
) -> Result<MarkdownConversion, String> {
    Ok(MarkdownConversion {
        document: parse_long_identifier(&raw.document)
            .ok_or_else(|| not_a_chained_identifier(ctx, "document", &raw.document))?,
        paragraph: parse_long_command('+', &raw.paragraph, ctx, "paragraph")?,
        hr: parse_long_command('+', &raw.hr, ctx, "hr")?,
        h1: parse_long_command('+', &raw.h1, ctx, "h1")?,
        h2: parse_long_command('+', &raw.h2, ctx, "h2")?,
        h3: parse_long_command('+', &raw.h3, ctx, "h3")?,
        h4: parse_long_command('+', &raw.h4, ctx, "h4")?,
        h5: parse_long_command('+', &raw.h5, ctx, "h5")?,
        h6: parse_long_command('+', &raw.h6, ctx, "h6")?,
        ul: parse_long_command('+', &raw.ul, ctx, "ul")?,
        ol: parse_long_command('+', &raw.ol, ctx, "ol")?,
        code_block: parse_long_command('+', &raw.code_block, ctx, "code_block")?,
        blockquote: parse_long_command('+', &raw.blockquote, ctx, "blockquote")?,
        emph: parse_long_command('\\', &raw.emph, ctx, "emph")?,
        strong: parse_long_command('\\', &raw.strong, ctx, "strong")?,
        hard_break: raw
            .hard_break
            .map(|s| parse_long_command('\\', &s, ctx, "hard_break"))
            .transpose()?,
        code: parse_long_command('\\', &raw.code, ctx, "code")?,
        link: parse_long_command('\\', &raw.link, ctx, "link")?,
        img: parse_long_command('\\', &raw.img, ctx, "img")?,
    })
}

fn not_a_chained_identifier(ctx: &str, field: &str, got: &str) -> String {
    format!("{ctx}.{field}: not a chained identifier: `{got}`")
}

/// `configUtil.ml:36-42` (`relative_path_decoder`): `Filename.is_relative`
/// on POSIX is "does not start with `/`"; the empty string is explicitly
/// allowed (upstream's own comment, `configUtil.ml:35`).
fn is_relative_path(s: &str) -> bool {
    !s.starts_with('/')
}

/// `commonUtil.ml:9-11` (`is_lowercased_identifier`): non-empty, first char
/// ASCII lowercase, every remaining char is `-` or ASCII alphanumeric.
fn is_lowercased_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c0) if c0.is_ascii_lowercase() => {
            chars.all(|c| c == '-' || c.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

/// `commonUtil.ml:6-9` (`is_uppercased_identifier`) — used here only for
/// the module-name components of a long command/identifier chain.
fn is_uppercased_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c0) if c0.is_ascii_uppercase() => {
            chars.all(|c| c == '-' || c.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

/// `commonUtil.ml:22-24` (`cut_module_names`): split on `.`, the last
/// component is the "variable"/command name, everything before it is the
/// module chain. `str::split('.')` on a dot-free string yields a single
/// one-element iterator, matching `String.split_on_char` here.
fn cut_module_names(s: &str) -> (Vec<String>, String) {
    let mut parts: Vec<&str> = s.split('.').collect();
    // `split` on any string (including "") always yields at least one
    // element, mirroring `String.split_on_char`'s non-empty-list guarantee.
    let name = parts.pop().expect("split always yields >=1 element");
    (parts.into_iter().map(String::from).collect(), name.to_string())
}

/// `commonUtil.ml:26-31` (`parse_long_command`): strip the `prefix` char,
/// then validate as a long identifier (every module component uppercased,
/// final component lowercased).
fn parse_long_command(
    prefix: char,
    s: &str,
    ctx: &str,
    field: &str,
) -> Result<LongCommand, String> {
    let mut chars = s.chars();
    let rest = if chars.next() == Some(prefix) {
        chars.as_str()
    } else {
        return Err(format!(
            "{ctx}.{field}: not a command starting with `{prefix}`: `{s}`"
        ));
    };
    let (modules, name) = cut_module_names(rest);
    if modules.iter().all(|m| is_uppercased_identifier(m)) && is_lowercased_identifier(&name) {
        Ok(LongCommand { modules, name })
    } else {
        Err(format!("{ctx}.{field}: not a command: `{s}`"))
    }
}

/// `commonUtil.ml:33-39` (`parse_long_identifier`): no prefix; same module/
/// name identifier-case validation as `parse_long_command`.
fn parse_long_identifier(s: &str) -> Option<LongIdentifier> {
    let (modules, name) = cut_module_names(s);
    if modules.iter().all(|m| is_uppercased_identifier(m)) && is_lowercased_identifier(&name) {
        Some(LongIdentifier { modules, name })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANNOT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v01x/envelope/annot.rustyfi-envelope.yaml"
    ));
    const MD_JA: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v01x/envelope/md-ja.rustyfi-envelope.yaml"
    ));
    const FONT_LATIN_MODERN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/v01x/envelope/font-latin-modern.rustyfi-envelope.yaml"
    ));

    /// e1: minimal library envelope (`annot`).
    #[test]
    fn envelope_annot_minimal_library_decodes() {
        let cfg = decode(ANNOT).expect("annot fixture decodes");
        match cfg.contents {
            EnvelopeContents::Library {
                main_module,
                source_directories,
                test_directories,
                markdown_conversion,
            } => {
                assert_eq!(main_module, "Annot");
                assert_eq!(source_directories, vec!["./src".to_string()]);
                assert!(test_directories.is_empty());
                assert!(markdown_conversion.is_none());
            }
            EnvelopeContents::Font { .. } => panic!("annot must decode as a library"),
        }
    }

    /// e2: library + full 19-field `markdown_conversion`, including a
    /// `null` (empty) `hard_break:` → `None` (`md-ja`).
    #[test]
    fn envelope_md_ja_markdown_conversion_decodes() {
        let cfg = decode(MD_JA).expect("md-ja fixture decodes");
        match cfg.contents {
            EnvelopeContents::Library {
                main_module,
                markdown_conversion,
                ..
            } => {
                assert_eq!(main_module, "MDJa");
                let md = markdown_conversion.expect("md-ja has markdown_conversion");
                assert_eq!(md.document.modules, vec!["MDJa".to_string()]);
                assert_eq!(md.document.name, "document");
                assert_eq!(md.paragraph.modules, vec!["MDJa".to_string()]);
                assert_eq!(md.paragraph.name, "p");
                assert_eq!(md.ul.name, "ul-block");
                assert_eq!(md.emph.name, "emph");
                assert_eq!(md.strong.name, "string");
                assert!(md.hard_break.is_none(), "hard_break: (null) must be None");
                assert_eq!(md.code.name, "code");
                assert_eq!(md.link.name, "link");
                assert_eq!(md.img.name, "img");
            }
            EnvelopeContents::Font { .. } => panic!("md-ja must decode as a library"),
        }
    }

    /// e3: font envelope, `opentype_single` only (`font-latin-modern`).
    #[test]
    fn envelope_font_latin_modern_decodes() {
        let cfg = decode(FONT_LATIN_MODERN).expect("font-latin-modern fixture decodes");
        match cfg.contents {
            EnvelopeContents::Font { main_module, files } => {
                assert_eq!(main_module, "FontLatinModern");
                assert_eq!(files.len(), 2);
                assert_eq!(files[0].path, "./fonts/lmmono10-regular.otf");
                match &files[0].contents {
                    FontFileContents::OpentypeSingle(spec) => {
                        assert_eq!(spec.name, "mono");
                        assert!(!spec.math);
                    }
                    FontFileContents::OpentypeCollection(_) => {
                        panic!("expected opentype_single")
                    }
                }
            }
            EnvelopeContents::Library { .. } => panic!("font-latin-modern must decode as a font"),
        }
    }

    /// e4: branch tolerance — an extra non-tag key beside `library:` is
    /// accepted (upstream `branch`, `yamlDecoder.ml:161-176`: only the tag
    /// keys among `fields` are inspected; anything else is ignored, same as
    /// `get`'s general unknown-key tolerance).
    #[test]
    fn envelope_extra_key_beside_tag_tolerated() {
        let yaml = "library:
  main_module: Foo
  source_directories: []
  test_directories: []
some_totally_unrelated_key: 42
";
        let cfg = decode(yaml).expect("extra sibling key must not break the branch decoder");
        assert!(matches!(cfg.contents, EnvelopeContents::Library { .. }));
    }

    /// e5: neither `library:` nor `font:` present → a branch_not_found-style
    /// error.
    #[test]
    fn envelope_neither_tag_errors() {
        let yaml = "main_module: Foo\n";
        let err = decode(yaml).expect_err("neither library nor font must error");
        assert!(err.contains("library"), "{err}");
        assert!(err.contains("font"), "{err}");
    }

    /// e6: both `library:` and `font:` present → a
    /// more_than_one_branch_found-style error.
    #[test]
    fn envelope_both_tags_errors() {
        let yaml = "library:
  main_module: Foo
  source_directories: []
  test_directories: []
font:
  main_module: Foo
  files: []
";
        let err = decode(yaml).expect_err("both library and font must error");
        assert!(err.contains("got both"), "{err}");
    }

    /// `opentype_collection` — no upstream fixture exercises this shape
    /// (§10.1 of the Ld3b spec); synthesized directly from the decoder
    /// shape (`envelopeConfig.ml:26-29`).
    #[test]
    fn envelope_opentype_collection_synthesized() {
        let yaml = "font:
  main_module: FontFoo
  files:
  - path: ./fonts/foo.ttc
    opentype_collection:
    - name: regular
      math: false
    - name: bold
      math: false
";
        let cfg = decode(yaml).expect("opentype_collection must decode");
        match cfg.contents {
            EnvelopeContents::Font { files, .. } => match &files[0].contents {
                FontFileContents::OpentypeCollection(specs) => {
                    assert_eq!(specs.len(), 2);
                    assert_eq!(specs[0].name, "regular");
                    assert_eq!(specs[1].name, "bold");
                }
                FontFileContents::OpentypeSingle(_) => panic!("expected opentype_collection"),
            },
            EnvelopeContents::Library { .. } => panic!("expected a font envelope"),
        }
    }

    /// Non-list `source_directories` is a structural (serde_yaml-level)
    /// decode error.
    #[test]
    fn envelope_non_list_source_directories_errors() {
        let yaml = "library:
  main_module: Foo
  source_directories: \"./src\"
  test_directories: []
";
        let err = decode(yaml).expect_err("non-list source_directories must error");
        assert!(!err.is_empty());
    }

    /// Absolute `files[].path` is rejected — the font path field, unlike
    /// `source_directories`, IS validated as relative.
    #[test]
    fn envelope_absolute_font_file_path_errors() {
        let yaml = "font:
  main_module: Foo
  files:
  - path: /abs/not/allowed.otf
    opentype_single:
      name: mono
      math: false
";
        let err = decode(yaml).expect_err("absolute font file path must error");
        assert!(err.contains("not a relative path"), "{err}");
    }

    /// Uppercase font `name` is rejected (lowercased identifier required).
    #[test]
    fn envelope_uppercase_font_name_errors() {
        let yaml = "font:
  main_module: Foo
  files:
  - path: ./fonts/foo.otf
    opentype_single:
      name: Mono
      math: false
";
        let err = decode(yaml).expect_err("uppercase font name must error");
        assert!(err.contains("not a lowercased identifier"), "{err}");
    }

    /// Bad `+`/`\` command shapes in `markdown_conversion`: wrong prefix,
    /// lowercase module component, uppercase final component.
    #[test]
    fn envelope_bad_markdown_command_shapes_error() {
        let base = |paragraph: &str, emph: &str| {
            format!(
                "library:
  main_module: Foo
  source_directories: []
  test_directories: []
  markdown_conversion:
    document: Foo.document
    paragraph: {paragraph}
    hr: +Foo.hr
    h1: +Foo.h1
    h2: +Foo.h2
    h3: +Foo.h3
    h4: +Foo.h4
    h5: +Foo.h5
    h6: +Foo.h6
    ul: +Foo.ul
    ol: +Foo.ol
    code_block: +Foo.code-block
    blockquote: +Foo.blockquote
    emph: {emph}
    strong: \\Foo.strong
    code: \\Foo.code
    link: \\Foo.link
    img: \\Foo.img
"
            )
        };

        // Wrong prefix on a block command (`\` instead of `+`).
        let yaml = base("\\Foo.p", "\\Foo.emph");
        assert!(decode(&yaml).is_err());

        // Lowercase module component.
        let yaml = base("+foo.p", "\\Foo.emph");
        assert!(decode(&yaml).is_err());

        // Uppercase final (command) component.
        let yaml = base("+Foo.P", "\\Foo.emph");
        assert!(decode(&yaml).is_err());

        // Sanity: the well-formed base case decodes.
        let yaml = base("+Foo.p", "\\Foo.emph");
        assert!(decode(&yaml).is_ok());
    }

    /// Reading a missing file surfaces the read-failure variant.
    #[test]
    fn envelope_file_missing_is_not_found() {
        let path =
            std::env::temp_dir().join("rustyfi-loader-envelope-test-does-not-exist.yaml");
        let err = load_config(&path).expect_err("missing file must error");
        assert!(matches!(err, LoadError::EnvelopeConfigNotFound { .. }));
    }

    /// `load_config` end-to-end (disk read + decode), against a real
    /// written copy of the `annot` fixture.
    #[test]
    fn envelope_load_config_reads_from_disk() {
        let path = std::env::temp_dir().join(format!(
            "rustyfi-loader-envelope-test-load-config-{}-{}.yaml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, ANNOT).expect("write temp envelope config");
        let result = load_config(&path);
        let _ = std::fs::remove_file(&path);
        let cfg = result.expect("annot fixture loads from disk");
        assert!(matches!(cfg.contents, EnvelopeContents::Library { .. }));
    }

    /// The full-corpus format gate (Ld3b spec §10.1/§11): when the vendored
    /// upstream checkout is present (env var `RUSTYFI_UPSTREAM_GIT`),
    /// `git show`s and decodes all 19 real
    /// `rustyfi-envelope.yaml.expected` files at
    /// `saphe-split @ b836d512689248d18970674021ecaca409e0d897`. Skipped
    /// (not failed) otherwise, since the vendored checkout is a local
    /// development convenience, not a repo dependency.
    #[test]
    fn envelope_sweep_all_19_upstream_fixtures_decode() {
        let Ok(upstream_git) = std::env::var("RUSTYFI_UPSTREAM_GIT") else {
            eprintln!(
                "skipping envelope_sweep_all_19_upstream_fixtures_decode: \
                 RUSTYFI_UPSTREAM_GIT is not set (see the Ld3b spec §10.1)"
            );
            return;
        };

        const PIN: &str = "b836d512689248d18970674021ecaca409e0d897";
        const PACKAGES: &[&str] = &[
            "annot/annot.0.0.1",
            "code/code.0.0.1",
            "font-ipa-ex/font-ipa-ex.0.0.1",
            "font-junicode/font-junicode.0.0.1",
            "font-latin-modern-math/font-latin-modern-math.0.0.1",
            "font-latin-modern/font-latin-modern.0.0.1",
            "footnote-scheme/footnote-scheme.0.0.1",
            "hyph-english/hyph-english.0.0.1",
            "itemize/itemize.0.0.1",
            "math/math.0.0.1",
            "md-ja/md-ja.0.0.1",
            "proof/proof.0.0.1",
            "std-ja-book/std-ja-book.0.0.1",
            "std-ja-report/std-ja-report.0.0.1",
            "std-ja/std-ja.0.0.1",
            "stdlib/stdlib.0.0.1",
            "tabular/tabular.0.0.1",
            "testing/testing.0.0.1",
            "unidata/unidata.0.0.1",
        ];
        assert_eq!(PACKAGES.len(), 19, "the sweep must cover all 19 packages");

        for pkg in PACKAGES {
            let rel = format!("lib-rustyfi/packages/{pkg}/rustyfi-envelope.yaml.expected");
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(&upstream_git)
                .arg("show")
                .arg(format!("{PIN}:{rel}"))
                .output()
                .unwrap_or_else(|e| panic!("failed to run `git show` for {rel}: {e}"));
            assert!(
                output.status.success(),
                "git show {PIN}:{rel} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout)
                .unwrap_or_else(|e| panic!("{rel}: not valid UTF-8: {e}"));
            decode(&text).unwrap_or_else(|e| panic!("{rel}: failed to decode: {e}"));
        }
    }
}
