//! The operator's branding: instance name, tagline, logo, dark logo, favicon
//! and the target of the attribution link.
//!
//! Configured in `[branding]` ([`BrandingConfig`]) and resolved once at
//! startup: the files are read, checked and kept in memory, each under a URL
//! derived from its content (`/branding/<16 hex digits of SHA-256>.<ext>`), so
//! a replaced file gets a new URL after a restart and the URL can be cached
//! for good. The files are served by [`crate::api::branding`] in both asset
//! modes; the shells reference them through [`Branding`].
//!
//! Only raster images are accepted: an SVG is an active document, and serving
//! one from the operator would need its own sandbox policy.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

use sha2::{Digest as _, Sha256};

use crate::config::{BrandingConfig, ConfigError, KNOWN_LANGUAGES};

/// Largest accepted branding file. Generous for a header logo or a favicon,
/// small in memory.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// The path below which branding files are served.
const URL_PREFIX: &str = "/branding/";

/// `[branding]` as the pages need it, with the files read and checked.
#[derive(Debug, Clone)]
pub struct Branding {
    /// The instance name, trimmed, not escaped.
    pub name: String,
    /// Show the logo without the name next to it (only with a logo).
    pub hide_name: bool,
    /// The slogan of each known language; a language without an entry has
    /// the built-in one. Read through [`Branding::tagline`].
    pub taglines: BTreeMap<&'static str, Tagline>,
    pub logo: Option<BrandingFile>,
    /// The logo for the dark theme (only with a logo).
    pub logo_dark: Option<BrandingFile>,
    /// `None`: the built-in favicon.
    pub favicon: Option<BrandingFile>,
    /// Target of the attribution link, not escaped.
    pub source_url: String,
}

/// The slogan next to the logo on the create page, in one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tagline {
    /// The built-in, translated slogan of the catalogue.
    BuiltIn,
    /// No slogan: the operator configured an empty tagline.
    Hidden,
    /// The operator's text, trimmed, not escaped.
    Text(String),
}

impl Tagline {
    /// A configured tagline: trimmed, empty hides the slogan.
    fn configured(value: &str) -> Self {
        match value.trim() {
            "" => Self::Hidden,
            text => Self::Text(text.to_owned()),
        }
    }
}

static BUILT_IN_TAGLINE: Tagline = Tagline::BuiltIn;

impl Branding {
    /// The slogan of the pages in `language`.
    pub fn tagline(&self, language: &str) -> &Tagline {
        self.taglines.get(language).unwrap_or(&BUILT_IN_TAGLINE)
    }

    /// Every configured file, a file configured twice (same content) once.
    pub fn files(&self) -> Vec<&BrandingFile> {
        let mut files: Vec<&BrandingFile> = Vec::new();
        for file in [&self.logo, &self.logo_dark, &self.favicon].into_iter().flatten() {
            if !files.iter().any(|known| known.url == file.url) {
                files.push(file);
            }
        }
        files
    }
}

/// One image file, read at startup.
#[derive(Debug, Clone)]
pub struct BrandingFile {
    /// `/branding/<16 hex digits of SHA-256>.<ext>`.
    pub url: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

/// An accepted file format: extension, MIME type and how its content starts.
#[derive(Debug, Clone, Copy)]
enum Format {
    Png,
    Webp,
    Ico,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Ico => "ico",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Webp => "WebP",
            Self::Ico => "ICO",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Ico => "image/x-icon",
        }
    }

    fn has_signature(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Self::Webp => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WEBP"[..]),
            Self::Ico => bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]),
        }
    }
}

/// Which files a key accepts, and how the error message names them.
const IMAGE_FORMATS: (&[Format], &str) = (&[Format::Png, Format::Webp], "PNG or WebP files (.png, .webp)");
const ICON_FORMATS: (&[Format], &str) = (&[Format::Png, Format::Ico], "PNG or ICO files (.png, .ico)");

/// Reads and checks the files of `[branding]`. Called by `Config::validate`
/// after `[branding]` itself was checked, so `capsulebin check` reports a
/// missing or wrong file too. Messages name the key and the path, never
/// anything read from the file.
pub fn resolve(config: &BrandingConfig) -> Result<Branding, ConfigError> {
    let file = |path: &Option<std::path::PathBuf>, key: &str, formats| path.as_deref().map(|path| read(path, key, formats)).transpose();
    Ok(Branding {
        name: config.name.trim().to_owned(),
        hide_name: config.hide_name,
        taglines: taglines(config),
        logo: file(&config.logo, "branding.logo", IMAGE_FORMATS)?,
        logo_dark: file(&config.logo_dark, "branding.logo_dark", IMAGE_FORMATS)?,
        favicon: file(&config.favicon, "branding.favicon", ICON_FORMATS)?,
        source_url: config.source_url.clone(),
    })
}

/// The tagline of every known language: its `[branding.<lang>]` tagline if
/// set, else `branding.tagline` if set, else the built-in one.
fn taglines(config: &BrandingConfig) -> BTreeMap<&'static str, Tagline> {
    KNOWN_LANGUAGES
        .iter()
        .map(|&language| {
            let configured = config
                .languages
                .get(language)
                .and_then(|texts| texts.tagline.as_deref())
                .or(config.tagline.as_deref());
            (language, configured.map_or(Tagline::BuiltIn, Tagline::configured))
        })
        .collect()
}

/// Reads one file. Relative paths are resolved against the working directory,
/// like `database.path`.
fn read(path: &Path, key: &str, (formats, accepted): (&[Format], &str)) -> Result<BrandingFile, ConfigError> {
    let invalid = |reason: String| ConfigError::Invalid(format!("{key}: {} {reason}", path.display()));

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let format = formats
        .iter()
        .copied()
        .find(|format| extension.as_deref() == Some(format.extension()))
        .ok_or_else(|| invalid(format!("has an unsupported format: only {accepted} are accepted")))?;

    // Read at most one byte more than allowed, so that a large file is not
    // loaded into memory just to be refused.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| invalid(format!("cannot be read: {error}")))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid(format!("is larger than {} KiB", MAX_FILE_BYTES / 1024)));
    }
    if !format.has_signature(&bytes) {
        return Err(invalid(format!("does not start with the {} signature", format.name())));
    }

    Ok(BrandingFile {
        url: format!("{URL_PREFIX}{}.{}", short_hash(&bytes), format.extension()),
        content_type: format.content_type(),
        bytes,
    })
}

/// The first 16 hex digits of the SHA-256 of `bytes`.
fn short_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)[..8].iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_need_their_whole_prefix() {
        assert!(Format::Webp.has_signature(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!Format::Webp.has_signature(b"RIFF\0\0\0\0WEB"));
        assert!(!Format::Webp.has_signature(b"RIFF\0\0\0\0WAVE"));
        assert!(Format::Ico.has_signature(&[0, 0, 1, 0, 1, 0]));
        assert!(!Format::Ico.has_signature(&[0, 0, 2, 0, 1, 0]));
        assert!(!Format::Png.has_signature(b"\x89PNG"));
    }

    fn tagline_config(tagline: Option<&str>, de: Option<&str>) -> BrandingConfig {
        let mut config = BrandingConfig {
            tagline: tagline.map(str::to_owned),
            ..BrandingConfig::default()
        };
        if let Some(de) = de {
            config.languages.insert(
                "de".to_owned(),
                crate::config::BrandingTexts {
                    tagline: Some(de.to_owned()),
                },
            );
        }
        config
    }

    /// The language's tagline, else `branding.tagline`, else the built-in
    /// slogan; empty (also after trimming) hides it.
    #[test]
    fn taglines_fall_back_from_the_language_to_branding_to_built_in() {
        let text = |text: &str| Tagline::Text(text.to_owned());
        for (tagline, de, expected_en, expected_de) in [
            (None, None, Tagline::BuiltIn, Tagline::BuiltIn),
            (Some(" Ours "), None, text("Ours"), text("Ours")),
            (None, Some("Unser"), Tagline::BuiltIn, text("Unser")),
            (Some("Ours"), Some("Unser"), text("Ours"), text("Unser")),
            (Some(""), None, Tagline::Hidden, Tagline::Hidden),
            (Some("Ours"), Some("  "), text("Ours"), Tagline::Hidden),
            (Some(""), Some("Unser"), Tagline::Hidden, text("Unser")),
            (None, Some(""), Tagline::BuiltIn, Tagline::Hidden),
        ] {
            let branding = resolve(&tagline_config(tagline, de)).expect("resolve");
            assert_eq!(branding.tagline("en"), &expected_en, "{tagline:?} {de:?}");
            assert_eq!(branding.tagline("de"), &expected_de, "{tagline:?} {de:?}");
        }
    }

    /// The operator's text is kept as written: escaping is the page's job.
    #[test]
    fn taglines_are_not_escaped() {
        let branding = resolve(&tagline_config(Some("A&B <i>%%capsulebin:name%%</i>"), None)).expect("resolve");
        assert_eq!(branding.tagline("en"), &Tagline::Text("A&B <i>%%capsulebin:name%%</i>".to_owned()));
    }

    #[test]
    fn an_unknown_language_has_the_built_in_tagline() {
        let branding = resolve(&tagline_config(Some("Ours"), None)).expect("resolve");
        assert_eq!(branding.tagline("fr"), &Tagline::BuiltIn);
    }

    #[test]
    fn the_hash_is_sixteen_hex_digits_of_sha256() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223...
        assert_eq!(short_hash(b"abc"), "ba7816bf8f01cfea");
    }
}
