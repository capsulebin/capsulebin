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

use crate::config::{Config, ConfigError, LegalSection, LegalSectionKind, language_prefix};

pub use markdown::render as render_markdown;

/// Largest legal text accepted, per text. Generous for any real imprint or
/// privacy notice, small enough that a wrong file fails at startup instead of
/// being sent with every request.
const MAX_TEXT_BYTES: usize = 64 * 1024;

/// Longest accepted `url`.
const MAX_URL_BYTES: usize = 2048;

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

/// One page served by CapsuleBin. Its title is the shell's: the footer label
/// of the page, from the catalogue of the shell's language.
#[derive(Debug, Clone)]
pub struct Page {
    /// The `<article>` with the rendered text, without a heading; `lang` marks
    /// what is not in the language of the shell it is served in.
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

    /// The page's path below the language prefix (if any), and its name in the
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
/// links to the pages served here carry its prefix (`/de/privacy`, or
/// `/privacy` for the default language). Called by
/// `Config::validate` for every known language, so `capsulebin check`
/// reports a missing file or a bad combination too.
///
/// Each section is `[legal.<language>.*]` if set, else `[legal.*]`; the
/// built-in texts are in `language`.
pub fn resolve(config: &Config, language: &str) -> Result<LegalPages, ConfigError> {
    let prefix = language_prefix(&config.i18n.default_language, language);
    let operator = effective(config, LegalSectionKind::Operator, language);
    let hosting = effective(config, LegalSectionKind::Hosting, language);

    let imprint = imprint(&operator, &hosting, language)?;
    let href = |which: Which, target: &Target| match target {
        Target::None => None,
        Target::Here(_) => Some(format!("{prefix}/{}", which.path())),
        Target::Elsewhere(url) => Some(url.clone()),
    };
    let mut facts = defaults::Facts::from_config(config, &prefix, href(Which::Imprint, &imprint), hosting.section.is_some());

    let privacy = document(
        &effective(config, LegalSectionKind::Privacy, language),
        &facts,
        defaults::privacy,
        language,
    )?;
    if let Target::Elsewhere(url) = &privacy {
        facts.privacy_href.clone_from(url);
    }
    let terms = document(
        &effective(config, LegalSectionKind::Terms, language),
        &facts,
        defaults::terms,
        language,
    )?;

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

/// The section as it applies to the pages of `language`: its own
/// `[legal.<language>.*]` as a whole if there is one, else `[legal.*]`.
fn effective<'a>(config: &'a Config, which: LegalSectionKind, language: &str) -> Effective<'a> {
    let legal = &config.legal;
    let own = legal.languages.get_key_value(language).and_then(|(language, texts)| {
        texts.section(which).map(|section| Effective {
            section: Some(section),
            language: language.as_str(),
            key: format!("legal.{language}.{}", which.name()),
        })
    });
    own.unwrap_or_else(|| Effective {
        section: legal.section(which),
        language: &config.i18n.default_language,
        key: format!("legal.{}", which.name()),
    })
}

/// A section that applies to the pages of one language.
struct Effective<'a> {
    section: Option<&'a LegalSection>,
    /// The language its texts are written in.
    language: &'a str,
    /// Where it is configured, for error messages (`legal.de.operator`).
    key: String,
}

/// A part of a page in one language, as Markdown.
struct Block<'a> {
    language: &'a str,
    markdown: String,
}

/// The imprint: the operator, then the hosting provider.
fn imprint(operator: &Effective, hosting: &Effective, language: &str) -> Result<Target, ConfigError> {
    let Some(operator_section) = operator.section else {
        if hosting.section.is_some() {
            return Err(invalid(&format!(
                "{} needs legal.operator: the imprint names the operator first",
                hosting.key
            )));
        }
        return Ok(Target::None);
    };
    reject_extra(operator_section, &operator.key)?;
    let operator_text = match source(operator_section, &operator.key, true)? {
        Source::Markdown(text) => text,
        Source::Url(url) => {
            if hosting.section.is_some() {
                return Err(invalid(&format!(
                    "{} needs {} as text or file: with a url the imprint is not served here",
                    hosting.key, operator.key
                )));
            }
            return Ok(Target::Elsewhere(url));
        }
        Source::Default => return Err(invalid(&format!("{} needs text, file or url", operator.key))),
    };

    let mut blocks = vec![Block {
        language: operator.language,
        markdown: operator_text,
    }];
    if let Some(hosting_section) = hosting.section {
        reject_extra(hosting_section, &hosting.key)?;
        match source(hosting_section, &hosting.key, false)? {
            // The heading belongs to the block, in its language.
            Source::Markdown(hosting_text) => blocks.push(Block {
                language: hosting.language,
                markdown: format!("## Hosting\n\n{hosting_text}"),
            }),
            Source::Url(_) | Source::Default => return Err(invalid(&format!("{} needs text or file", hosting.key))),
        }
    }
    Ok(Target::Here(page(&blocks, language)))
}

/// The privacy notice or the terms: the operator's own, or the built-in text
/// in `language` with the operator's additions.
fn document(
    effective: &Effective,
    facts: &defaults::Facts,
    built_in: fn(&defaults::Facts, &str) -> defaults::BuiltIn,
    language: &str,
) -> Result<Target, ConfigError> {
    let key = &effective.key;
    let section = effective.section.cloned().unwrap_or_default();
    match source(&section, key, true)? {
        Source::Url(url) => {
            reject_extra(&section, key)?;
            Ok(Target::Elsewhere(url))
        }
        Source::Markdown(text) => {
            reject_extra(&section, key)?;
            let block = Block {
                language: effective.language,
                markdown: text,
            };
            Ok(Target::Here(page(&[block], language)))
        }
        Source::Default => {
            let extra = extra(&section, key)?;
            if facts.imprint_href.is_none() {
                // Without an operator the built-in text would have no controller.
                return match extra {
                    Some(_) => Err(invalid(&format!(
                        "{key}.extra extends the built-in text, which needs legal.operator"
                    ))),
                    None => Ok(Target::None),
                };
            }
            let text = built_in(facts, language);
            let blocks = match extra {
                // The extra in another language is a block of its own, so it can be marked.
                Some(extra) if effective.language != language => vec![
                    Block {
                        language,
                        markdown: text.before_extra,
                    },
                    Block {
                        language: effective.language,
                        markdown: extra.trim().to_owned(),
                    },
                    Block {
                        language,
                        markdown: text.after_extra,
                    },
                ],
                extra => vec![Block {
                    language,
                    markdown: text.join(extra.as_deref()),
                }],
            };
            Ok(Target::Here(page(&blocks, language)))
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
        return Err(invalid(&format!("{key}: set one of text, file and url, not several")));
    }
    if let Some(text) = &section.text {
        return Ok(Source::Markdown(checked_text(text.clone(), &format!("{key}.text"))?));
    }
    if let Some(path) = &section.file {
        return Ok(Source::Markdown(read(path, &format!("{key}.file"))?));
    }
    if let Some(url) = &section.url {
        if !url_allowed {
            return Err(invalid(&format!("{key}.url is not supported, use text or file")));
        }
        if !is_acceptable_url(url) {
            return Err(invalid(&format!(
                "{key}.url must be an absolute http(s) URL or a path starting with a single /, \
                 at most {MAX_URL_BYTES} bytes, without spaces or control characters; got {url:?}"
            )));
        }
        return Ok(Source::Url(url.clone()));
    }
    Ok(Source::Default)
}

fn extra(section: &LegalSection, key: &str) -> Result<Option<String>, ConfigError> {
    match (&section.extra, &section.extra_file) {
        (Some(_), Some(_)) => Err(invalid(&format!("{key}: set extra or extra_file, not both"))),
        (Some(text), None) => checked_text(text.clone(), &format!("{key}.extra")).map(Some),
        (None, Some(path)) => read(path, &format!("{key}.extra_file")).map(Some),
        (None, None) => Ok(None),
    }
}

fn reject_extra(section: &LegalSection, key: &str) -> Result<(), ConfigError> {
    if section.extra.is_some() || section.extra_file.is_some() {
        return Err(invalid(&format!(
            "{key}.extra only extends a built-in text; put the whole text into text or file instead"
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

/// A page for the shell of `language`. `lang` lets a screen reader pronounce
/// a text in another language correctly: on the `<article>` when the whole
/// text is in one other language, else on each block that is not in
/// `language`. Nothing is marked that is in the shell's language.
fn page(blocks: &[Block], language: &str) -> Page {
    let lang = |code: &str| format!(r#" lang="{}""#, escape(code));
    let foreign = |block: &&Block| block.language != language;
    let whole = blocks
        .first()
        .filter(|first| blocks.iter().all(|block| block.language == first.language))
        .filter(foreign);
    let html = match whole {
        Some(first) => format!(
            "<article{}>{}</article>",
            lang(first.language),
            blocks.iter().map(|block| render_markdown(&block.markdown)).collect::<String>()
        ),
        None => format!(
            "<article>{}</article>",
            blocks
                .iter()
                .map(|block| {
                    let html = render_markdown(&block.markdown);
                    if foreign(&block) {
                        format!("<div{}>{html}</div>", lang(block.language))
                    } else {
                        html
                    }
                })
                .collect::<String>()
        ),
    };
    Page { html }
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
