//! The operator's legal pages: imprint, privacy notice, terms of use.
//!
//! Configured in `[legal]` ([`LegalConfig`]) and resolved once at startup:
//! files are read, Markdown is rendered and the built-in texts are filled in
//! from the configuration, so a request only copies finished HTML into the
//! `legal.html` shell ([`crate::api::assets`]).
//!
//! This is the one place where the server puts content into a page: operator
//! text, never capsule content, and never code. Every script and stylesheet
//! still comes from the binary; the rendered Markdown carries no raw HTML and
//! only `http(s)`, `mailto` and `tel` links, and the CSP would refuse an inline
//! script anyway (ADR-015).

mod defaults;
mod markdown;

use std::path::Path;

use crate::config::{Config, ConfigError, LegalSection};

pub use markdown::render as render_markdown;

/// Largest legal text accepted, per text. Generous for any real imprint or
/// privacy notice, small enough that a wrong file fails at startup instead of
/// being sent with every request.
const MAX_TEXT_BYTES: usize = 64 * 1024;

/// Longest accepted `url`.
const MAX_URL_BYTES: usize = 2048;

/// The language of the legal texts: the built-in ones are English, and the
/// operator's text for a page is served under every language prefix.
const TEXT_LANGUAGE: &str = "en";

/// Everything the legal routes and the footer of one interface language need,
/// rendered.
#[derive(Debug, Clone, Default)]
pub struct LegalPages {
    /// Where the footer links point; `None` omits a link. The labels come
    /// from the shell, written by the web build in its language.
    pub footer: FooterLinks,
    pub imprint: Option<Page>,
    pub privacy: Option<Page>,
    pub terms: Option<Page>,
}

/// The target of each footer link, `None` for a page that is not configured.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FooterLinks {
    pub imprint: Option<String>,
    pub privacy: Option<String>,
    pub terms: Option<String>,
}

impl FooterLinks {
    pub fn href(&self, which: Which) -> Option<&str> {
        match which {
            Which::Imprint => self.imprint.as_deref(),
            Which::Privacy => self.privacy.as_deref(),
            Which::Terms => self.terms.as_deref(),
        }
    }
}

/// One page served by CapsuleBin.
#[derive(Debug, Clone)]
pub struct Page {
    /// Plain text, for `<title>` and the header.
    pub title: &'static str,
    /// The `<article>` with heading and rendered text; it carries `lang` when
    /// the text is not in the language of the shell it is served in.
    pub html: String,
}

/// Which legal page a route serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Imprint,
    Privacy,
    Terms,
}

impl Which {
    pub const ALL: [Self; 3] = [Self::Imprint, Self::Privacy, Self::Terms];

    /// The page's path below the language prefix, and its name in the
    /// shell's footer marker.
    pub fn path(self) -> &'static str {
        match self {
            Self::Imprint => "imprint",
            Self::Privacy => "privacy",
            Self::Terms => "terms",
        }
    }
}

impl LegalPages {
    pub fn page(&self, which: Which) -> Option<&Page> {
        match which {
            Which::Imprint => self.imprint.as_ref(),
            Which::Privacy => self.privacy.as_ref(),
            Which::Terms => self.terms.as_ref(),
        }
    }
}

/// Where a footer entry points, if anywhere.
enum Target {
    None,
    Here(Page),
    Elsewhere(String),
}

/// Reads, checks and renders the legal pages for the shells of `language`:
/// links to the pages served here carry its prefix (`/de/privacy`). Called by
/// `Config::validate` for every known language, so `capsulebin check`
/// reports a missing file or a bad combination too.
pub fn resolve(config: &Config, language: &str) -> Result<LegalPages, ConfigError> {
    let legal = &config.legal;
    let prefix = format!("/{language}");
    let page = |title: &'static str, markdown: &str| page(title, markdown, language);

    if legal.hosting.is_some() && legal.operator.is_none() {
        return Err(invalid("legal.hosting needs legal.operator: the imprint names the operator first"));
    }

    // Imprint: operator, then hosting.
    let imprint = match &legal.operator {
        None => Target::None,
        Some(operator) => {
            reject_extra(operator, "operator")?;
            match source(operator, "operator", true)? {
                Source::Url(url) => {
                    if legal.hosting.is_some() {
                        return Err(invalid(
                            "legal.hosting needs legal.operator as text or file: with a url the imprint is not served here",
                        ));
                    }
                    Target::Elsewhere(url)
                }
                Source::Markdown(operator_text) => {
                    let mut markdown = operator_text;
                    if let Some(hosting) = &legal.hosting {
                        reject_extra(hosting, "hosting")?;
                        match source(hosting, "hosting", false)? {
                            Source::Markdown(hosting_text) => {
                                markdown.push_str(&format!("\n\n## Hosting\n\n{hosting_text}"));
                            }
                            Source::Url(_) | Source::Default => {
                                return Err(invalid("legal.hosting needs text or file"));
                            }
                        }
                    }
                    Target::Here(page("Imprint", &markdown))
                }
                Source::Default => return Err(invalid("legal.operator needs text, file or url")),
            }
        }
    };

    let href = |which: Which, target: &Target| match target {
        Target::None => None,
        Target::Here(_) => Some(format!("{prefix}/{}", which.path())),
        Target::Elsewhere(url) => Some(url.clone()),
    };
    let mut facts = defaults::Facts::from_config(config, &prefix, href(Which::Imprint, &imprint), legal.hosting.is_some());

    let privacy = document(
        legal.privacy.as_ref(),
        "privacy",
        "Privacy notice",
        &facts,
        defaults::privacy,
        &page,
    )?;
    if let Target::Elsewhere(url) = &privacy {
        facts.privacy_href.clone_from(url);
    }
    let terms = document(legal.terms.as_ref(), "terms", "Terms of use", &facts, defaults::terms, &page)?;

    let footer = FooterLinks {
        imprint: href(Which::Imprint, &imprint),
        privacy: href(Which::Privacy, &privacy),
        terms: href(Which::Terms, &terms),
    };

    let served = |target: Target| match target {
        Target::Here(page) => Some(page),
        Target::None | Target::Elsewhere(_) => None,
    };
    Ok(LegalPages {
        footer,
        imprint: served(imprint),
        privacy: served(privacy),
        terms: served(terms),
    })
}

/// The privacy notice or the terms: the operator's own, or the built-in text
/// with the operator's additions.
fn document(
    section: Option<&LegalSection>,
    key: &str,
    title: &'static str,
    facts: &defaults::Facts,
    built_in: fn(&defaults::Facts, Option<&str>) -> String,
    page: &dyn Fn(&'static str, &str) -> Page,
) -> Result<Target, ConfigError> {
    let section_or_default = section.cloned().unwrap_or_default();
    match source(&section_or_default, key, true)? {
        Source::Url(url) => {
            reject_extra(&section_or_default, key)?;
            Ok(Target::Elsewhere(url))
        }
        Source::Markdown(text) => {
            reject_extra(&section_or_default, key)?;
            Ok(Target::Here(page(title, &text)))
        }
        Source::Default => {
            let extra = extra(&section_or_default, key)?;
            if facts.imprint_href.is_none() {
                // Without an operator the built-in text would have no controller.
                return match extra {
                    Some(_) => Err(invalid(&format!(
                        "legal.{key}.extra extends the built-in text, which needs legal.operator"
                    ))),
                    None => Ok(Target::None),
                };
            }
            Ok(Target::Here(page(title, &built_in(facts, extra.as_deref()))))
        }
    }
}

enum Source {
    Markdown(String),
    Url(String),
    /// Neither text, file nor url.
    Default,
}

fn source(section: &LegalSection, key: &str, url_allowed: bool) -> Result<Source, ConfigError> {
    let given = [section.text.is_some(), section.file.is_some(), section.url.is_some()]
        .into_iter()
        .filter(|set| *set)
        .count();
    if given > 1 {
        return Err(invalid(&format!("legal.{key}: set one of text, file and url, not several")));
    }
    if let Some(text) = &section.text {
        return Ok(Source::Markdown(checked_text(text.clone(), &format!("legal.{key}.text"))?));
    }
    if let Some(path) = &section.file {
        return Ok(Source::Markdown(read(path, &format!("legal.{key}.file"))?));
    }
    if let Some(url) = &section.url {
        if !url_allowed {
            return Err(invalid(&format!("legal.{key}.url is not supported, use text or file")));
        }
        if !is_acceptable_url(url) {
            return Err(invalid(&format!(
                "legal.{key}.url must be an absolute http(s) URL or a path starting with a single /, \
                 at most {MAX_URL_BYTES} bytes, without spaces or control characters; got {url:?}"
            )));
        }
        return Ok(Source::Url(url.clone()));
    }
    Ok(Source::Default)
}

fn extra(section: &LegalSection, key: &str) -> Result<Option<String>, ConfigError> {
    match (&section.extra, &section.extra_file) {
        (Some(_), Some(_)) => Err(invalid(&format!("legal.{key}: set extra or extra_file, not both"))),
        (Some(text), None) => checked_text(text.clone(), &format!("legal.{key}.extra")).map(Some),
        (None, Some(path)) => read(path, &format!("legal.{key}.extra_file")).map(Some),
        (None, None) => Ok(None),
    }
}

fn reject_extra(section: &LegalSection, key: &str) -> Result<(), ConfigError> {
    if section.extra.is_some() || section.extra_file.is_some() {
        return Err(invalid(&format!(
            "legal.{key}.extra only extends a built-in text; put the whole text into text or file instead"
        )));
    }
    Ok(())
}

fn read(path: &Path, key: &str) -> Result<String, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|error| invalid(&format!("{key}: cannot read {}: {error}", path.display())))?;
    checked_text(text, key)
}

fn checked_text(text: String, key: &str) -> Result<String, ConfigError> {
    if text.trim().is_empty() {
        return Err(invalid(&format!("{key} is empty")));
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(invalid(&format!("{key} is larger than {} KiB", MAX_TEXT_BYTES / 1024)));
    }
    Ok(text)
}

/// An absolute http(s) URL, or a path on this host (for pages a reverse proxy
/// serves next to CapsuleBin).
///
/// Everything else is refused because it ends up in an `href` on every page:
/// `javascript:` and `data:` would run or render on our origin, and `//host` or
/// `/\host` are protocol-relative and leave the site although they look like
/// paths.
fn is_acceptable_url(url: &str) -> bool {
    if url.is_empty() || url.len() > MAX_URL_BYTES || url.contains('\\') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let absolute = ["https://", "http://"].iter().any(|scheme| {
        lower
            .strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
    });
    let local_path = url.starts_with('/') && !url.starts_with("//");
    absolute || local_path
}

/// A page for the shell of `language`. The `lang` attribute lets a screen
/// reader pronounce an English text in a German shell as English.
fn page(title: &'static str, markdown: &str, language: &str) -> Page {
    let lang = if language == TEXT_LANGUAGE {
        String::new()
    } else {
        format!(r#" lang="{TEXT_LANGUAGE}""#)
    };
    Page {
        title,
        html: format!(
            r#"<article class="cb-prose cb-legal"{lang}><h1>{}</h1>{}</article>"#,
            escape(title),
            render_markdown(markdown)
        ),
    }
}

/// HTML escaping for text and attribute values.
pub(crate) fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn invalid(message: &str) -> ConfigError {
    ConfigError::Invalid(message.to_owned())
}

#[cfg(test)]
mod tests;
