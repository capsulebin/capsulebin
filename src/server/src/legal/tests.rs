//! Tests for resolving `[legal]`: which pages exist, what the footer links to,
//! which combinations are refused, and what the built-in texts state.

use super::*;
use crate::config::{Config, LegalSection, LegalTexts};

fn text(markdown: &str) -> Option<LegalSection> {
    Some(LegalSection {
        text: Some(markdown.to_owned()),
        ..LegalSection::default()
    })
}

fn url(href: &str) -> Option<LegalSection> {
    Some(LegalSection {
        url: Some(href.to_owned()),
        ..LegalSection::default()
    })
}

fn with_operator() -> Config {
    let mut config = Config::default();
    config.legal.operator = text("Example GmbH  \nStreet 1, 12345 City");
    config
}

fn links(imprint: Option<&str>, privacy: Option<&str>, terms: Option<&str>) -> FooterLinks {
    FooterLinks {
        imprint: imprint.map(str::to_owned),
        privacy: privacy.map(str::to_owned),
        terms: terms.map(str::to_owned),
    }
}

fn resolved(config: &Config) -> LegalPages {
    resolve(config, "en").expect("legal configuration must resolve")
}

fn refused(config: &Config) -> String {
    match resolve(config, "en") {
        Ok(_) => panic!("configuration was accepted"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn nothing_configured_means_no_pages_and_no_links() {
    let pages = resolved(&Config::default());

    assert_eq!(pages.footer, FooterLinks::default());
    assert!(pages.imprint.is_none() && pages.privacy.is_none() && pages.terms.is_none());
}

#[test]
fn an_operator_brings_the_imprint_and_both_built_in_texts() {
    let pages = resolved(&with_operator());

    assert_eq!(pages.footer, links(Some("/imprint"), Some("/privacy"), Some("/terms")));
    let imprint = pages.imprint.expect("imprint");
    assert!(imprint.html.contains("Example GmbH<br />"), "{}", imprint.html);
    assert!(pages.privacy.expect("privacy").html.contains("<h2>Controller</h2>"));
    assert!(pages.terms.expect("terms").html.contains("<h2>What you may not share</h2>"));
}

/// Links to pages served here carry the prefix of the shell they are shown in,
/// none for the default language; links elsewhere stay as configured. The
/// built-in texts are in the shell's language; the operator's text stays in
/// the default language, marked as such inside a shell of another language.
#[test]
fn links_follow_the_language_of_the_shell() {
    let mut config = with_operator();
    config.legal.privacy = url("/privacy-notice");
    let pages = resolve(&config, "de").expect("legal configuration must resolve");

    assert_eq!(pages.footer, links(Some("/de/imprint"), Some("/privacy-notice"), Some("/de/terms")));
    let terms = pages.terms.expect("terms").html;
    assert!(terms.starts_with("<article><p>Mit diesem Dienst teilst du"), "{terms}");
    assert!(terms.contains(r#"href="/de/report""#), "{terms}");
    assert!(pages.imprint.expect("imprint").html.starts_with(r#"<article lang="en">"#));

    let english = resolved(&config).terms.expect("terms").html;
    assert!(english.starts_with("<article><p>This service lets you share"), "{english}");
    assert!(english.contains(r#"href="/report""#), "{english}");

    // With German as the default it is English that has the prefix.
    config.i18n.default_language = "de".to_owned();
    let pages = resolve(&config, "de").expect("legal configuration must resolve");
    assert_eq!(pages.footer, links(Some("/imprint"), Some("/privacy-notice"), Some("/terms")));
    assert!(pages.terms.expect("terms").html.contains(r#"href="/report""#));
    assert!(pages.imprint.expect("imprint").html.starts_with("<article>"));
    let pages = resolved(&config);
    assert_eq!(pages.footer, links(Some("/en/imprint"), Some("/privacy-notice"), Some("/en/terms")));
    assert!(pages.terms.expect("terms").html.contains(r#"href="/en/report""#));
    assert!(pages.imprint.expect("imprint").html.starts_with(r#"<article lang="de">"#));
}

#[test]
fn the_built_in_privacy_notice_links_its_imprint_under_the_prefix() {
    let privacy = resolve(&with_operator(), "de").expect("resolve").privacy.expect("privacy").html;
    assert!(privacy.contains(r#"href="/de/imprint""#), "{privacy}");
}

/// AC18: what the browser keeps is named, key by key.
#[test]
fn the_built_in_privacy_notice_names_what_the_browser_stores() {
    let privacy = resolved(&with_operator()).privacy.expect("privacy").html;
    assert!(privacy.contains("<code>capsulebin.theme</code>"), "{privacy}");
    assert!(privacy.contains("<code>capsulebin.language</code>"), "{privacy}");
}

#[test]
fn hosting_is_a_second_block_in_the_imprint_and_a_processor_in_the_privacy_notice() {
    let mut config = with_operator();
    config.legal.hosting = text("Hoster AG, Server Street 2");
    let pages = resolved(&config);

    let imprint = pages.imprint.expect("imprint").html;
    let operator = imprint.find("Example GmbH").expect("operator");
    let hosting = imprint.find("<h2>Hosting</h2>").expect("hosting heading");
    assert!(operator < hosting && imprint.contains("Hoster AG"), "{imprint}");
    assert!(pages.privacy.expect("privacy").html.contains("Art. 28 GDPR"));

    // Without hosting, no processor is claimed.
    assert!(!resolved(&with_operator()).privacy.expect("privacy").html.contains("Art. 28"));
}

#[test]
fn the_built_in_privacy_notice_follows_the_configuration() {
    let mut config = with_operator();
    config.purge.never_retrieved_after = crate::config::units::HumanDuration::parse("14d").expect("duration");
    config.abuse.report_retention = crate::config::units::HumanDuration::parse("30d").expect("duration");
    let privacy = resolved(&config).privacy.expect("privacy").html;
    assert!(
        privacy.contains("14 days after it was created if it was never retrieved"),
        "{privacy}"
    );
    assert!(privacy.contains("deleted after 30 days"), "{privacy}");
    assert!(privacy.contains("does not store IP addresses"), "{privacy}");

    config.rate_limits.create.requests_per_minute = 5;
    config.rate_limits.create.burst = 2;
    let privacy = resolved(&config).privacy.expect("privacy").html;
    assert!(privacy.contains("processes your IP address in its memory"), "{privacy}");

    config.purge.enabled = false;
    config.abuse.enabled = false;
    let privacy = resolved(&config).privacy.expect("privacy").html;
    assert!(!privacy.contains("if it was never retrieved"), "{privacy}");
    assert!(!privacy.contains("<h2>Reports</h2>"), "{privacy}");
}

#[test]
fn extra_is_added_to_the_built_in_text_before_the_rights() {
    let mut config = with_operator();
    config.legal.privacy = Some(LegalSection {
        extra: Some("## Web server logs\n\nOur proxy keeps IP addresses for 7 days.".to_owned()),
        ..LegalSection::default()
    });
    let privacy = resolved(&config).privacy.expect("privacy").html;

    let extra = privacy.find("<h2>Web server logs</h2>").expect("extra");
    let rights = privacy.find("<h2>Your rights</h2>").expect("rights");
    assert!(extra < rights, "{privacy}");
}

#[test]
fn own_texts_replace_the_built_in_ones() {
    let mut config = with_operator();
    config.legal.privacy = text("Our own notice.");
    config.legal.terms = text("Our own terms.");
    let pages = resolved(&config);

    let privacy = pages.privacy.expect("privacy").html;
    assert!(privacy.contains("Our own notice.") && !privacy.contains("Controller"), "{privacy}");
    assert!(pages.terms.expect("terms").html.contains("Our own terms."));
}

#[test]
fn urls_link_out_and_serve_no_page() {
    let mut config = Config::default();
    config.legal.operator = url("https://operator.example/imprint");
    config.legal.privacy = url("https://operator.example/privacy");
    let pages = resolved(&config);

    assert!(pages.imprint.is_none() && pages.privacy.is_none());
    assert_eq!(pages.footer.imprint.as_deref(), Some("https://operator.example/imprint"));
    assert_eq!(pages.footer.privacy.as_deref(), Some("https://operator.example/privacy"));
    // The built-in terms point to the controller and the privacy notice where they are.
    let terms = pages.terms.expect("terms").html;
    assert!(terms.contains(r#"href="https://operator.example/privacy""#), "{terms}");
}

#[test]
fn privacy_and_terms_of_their_own_need_no_operator() {
    let mut config = Config::default();
    config.legal.privacy = url("https://operator.example/privacy");
    let pages = resolved(&config);

    assert_eq!(pages.footer, links(None, Some("https://operator.example/privacy"), None));
    assert!(pages.terms.is_none(), "built-in terms without an operator");
}

#[test]
fn files_are_read_at_startup() {
    let dir = std::env::temp_dir().join(format!("capsulebin-legal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("operator.md");
    std::fs::write(&file, "From a **file**.").expect("write");

    let mut config = Config::default();
    config.legal.operator = Some(LegalSection {
        file: Some(file.clone()),
        ..LegalSection::default()
    });
    assert!(
        resolved(&config)
            .imprint
            .expect("imprint")
            .html
            .contains("From a <strong>file</strong>.")
    );

    config.legal.operator = Some(LegalSection {
        file: Some(dir.join("missing.md")),
        ..LegalSection::default()
    });
    assert!(refused(&config).contains("legal.operator.file: cannot read"));

    let _ = std::fs::remove_file(&file);
}

#[test]
fn refuses_combinations_that_cannot_be_meant() {
    type Adjust = Box<dyn Fn(&mut Config)>;
    let cases: Vec<(&str, Adjust)> = vec![
        (
            "hosting without operator",
            Box::new(|c| {
                c.legal.operator = None;
                c.legal.hosting = text("Hoster");
            }),
        ),
        (
            "hosting with an operator url",
            Box::new(|c| {
                c.legal.operator = url("https://operator.example/imprint");
                c.legal.hosting = text("Hoster");
            }),
        ),
        ("hosting as url", Box::new(|c| c.legal.hosting = url("https://hoster.example"))),
        ("empty operator", Box::new(|c| c.legal.operator = Some(LegalSection::default()))),
        ("blank text", Box::new(|c| c.legal.operator = text("  \n"))),
        (
            "text and url",
            Box::new(|c| {
                c.legal.terms = Some(LegalSection {
                    text: Some("a".to_owned()),
                    url: Some("https://x.example".to_owned()),
                    ..LegalSection::default()
                })
            }),
        ),
        (
            "extra on an own text",
            Box::new(|c| {
                c.legal.privacy = Some(LegalSection {
                    text: Some("a".to_owned()),
                    extra: Some("b".to_owned()),
                    ..LegalSection::default()
                })
            }),
        ),
        (
            "extra on the operator",
            Box::new(|c| {
                c.legal.operator = Some(LegalSection {
                    text: Some("a".to_owned()),
                    extra: Some("b".to_owned()),
                    ..LegalSection::default()
                })
            }),
        ),
        ("too long", Box::new(|c| c.legal.operator = text(&"a".repeat(MAX_TEXT_BYTES + 1)))),
    ];

    for (name, adjust) in cases {
        let mut config = with_operator();
        adjust(&mut config);
        assert!(resolve(&config, "en").is_err(), "{name} accepted");
    }

    // extra without operator: the built-in text it extends would have no controller.
    let mut config = Config::default();
    config.legal.terms = Some(LegalSection {
        extra: Some("b".to_owned()),
        ..LegalSection::default()
    });
    assert!(refused(&config).contains("needs legal.operator"));
}

/// Each of these would land in an `href` on every page.
#[test]
fn refuses_urls_that_run_script_or_leave_the_site_unnoticed() {
    for href in [
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "//evil.example/imprint",
        "/\\evil.example/imprint",
        "https:/evil.example",
        "https://",
        "https:///path",
        "imprint.html",
        "https://operator.example/im print",
        "https://operator.example/imprint\n",
        &format!("https://operator.example/{}", "a".repeat(MAX_URL_BYTES)),
    ] {
        let mut config = Config::default();
        config.legal.privacy = url(href);
        assert!(resolve(&config, "en").is_err(), "{href:?} accepted");
    }

    for href in [
        "https://operator.example/privacy",
        "HTTP://operator.example/p?x=1#top",
        "/privacy-notice",
    ] {
        let mut config = Config::default();
        config.legal.privacy = url(href);
        assert!(resolve(&config, "en").is_ok(), "{href:?} refused");
    }
}

fn german() -> LegalTexts {
    LegalTexts::default()
}

fn with_german(config: &mut Config, adjust: impl FnOnce(&mut LegalTexts)) {
    adjust(config.legal.languages.entry("de".to_owned()).or_insert_with(german));
}

/// The values of the `lang` attributes of a page, in order.
fn langs(html: &str) -> Vec<&str> {
    html.split(" lang=\"").skip(1).filter_map(|rest| rest.split('"').next()).collect()
}

/// AC4: an `extra` in the default language is the one marked block of a
/// German built-in text, between its two parts.
#[test]
fn an_extra_in_another_language_is_marked_on_its_own() {
    let mut config = with_operator();
    config.legal.privacy = Some(LegalSection {
        extra: Some("## Web server logs\n\nOur proxy keeps IP addresses for 7 days.".to_owned()),
        ..LegalSection::default()
    });
    let privacy = resolve(&config, "de").expect("resolve").privacy.expect("privacy").html;

    assert!(privacy.starts_with("<article><h2>Verantwortlicher</h2>"), "{privacy}");
    assert_eq!(langs(&privacy), ["en"], "{privacy}");
    let extra = privacy
        .find("<div lang=\"en\"><h2>Web server logs</h2>\n<p>Our proxy keeps IP addresses for 7 days.</p>\n</div>")
        .expect("the extra in its own block");
    let rights = privacy.find("<h2>Deine Rechte</h2>").expect("rights");
    assert!(extra < rights, "{privacy}");

    // In the default language's pages nothing is marked.
    let english = resolved(&config).privacy.expect("privacy").html;
    assert!(
        langs(&english).is_empty() && english.contains("<h2>Web server logs</h2>"),
        "{english}"
    );
}

/// AC4: the operator's German text is unmarked in the German imprint, the
/// hosting block falling back to the default language is marked, heading
/// included.
#[test]
fn a_hosting_block_in_another_language_is_marked_on_its_own() {
    let mut config = with_operator();
    config.legal.hosting = text("Hoster AG");
    with_german(&mut config, |de| de.operator = text("Beispiel GmbH"));
    let imprint = resolve(&config, "de").expect("resolve").imprint.expect("imprint").html;

    assert_eq!(
        imprint,
        "<article><p>Beispiel GmbH</p>\n<div lang=\"en\"><h2>Hosting</h2>\n<p>Hoster AG</p>\n</div></article>"
    );
    // Each language's own imprint where both are set.
    with_german(&mut config, |de| de.hosting = text("Hoster AG, Deutschland"));
    let imprint = resolve(&config, "de").expect("resolve").imprint.expect("imprint").html;
    assert!(
        langs(&imprint).is_empty() && imprint.contains("Hoster AG, Deutschland"),
        "{imprint}"
    );
    let english = resolved(&config).imprint.expect("imprint").html;
    assert!(langs(&english).is_empty() && english.contains("Example GmbH"), "{english}");
}

/// AC5: a language's section replaces the default one as a whole.
#[test]
fn a_language_section_replaces_the_default_one_as_a_whole() {
    let mut config = with_operator();
    config.legal.privacy = text("Our own notice.");
    config.legal.terms = Some(LegalSection {
        extra: Some("English extra.".to_owned()),
        ..LegalSection::default()
    });
    with_german(&mut config, |de| {
        de.privacy = Some(LegalSection::default());
        de.terms = Some(LegalSection {
            extra: Some("Deutscher Zusatz.".to_owned()),
            ..LegalSection::default()
        });
    });
    let pages = resolve(&config, "de").expect("resolve");

    let privacy = pages.privacy.expect("privacy").html;
    assert!(
        privacy.contains("<h2>Verantwortlicher</h2>") && !privacy.contains("Our own notice."),
        "{privacy}"
    );
    let terms = pages.terms.expect("terms").html;
    assert!(
        terms.contains("<p>Deutscher Zusatz.</p>") && !terms.contains("English extra."),
        "{terms}"
    );
    assert!(langs(&terms).is_empty(), "{terms}");

    let english = resolved(&config);
    assert!(english.privacy.expect("privacy").html.contains("Our own notice."));
    assert!(english.terms.expect("terms").html.contains("English extra."));
}

/// AC7: a `url` per language decides the footer and the controller of the
/// built-in privacy notice in that language.
#[test]
fn urls_are_per_language() {
    let mut config = Config::default();
    config.legal.operator = url("https://example.org/imprint");
    with_german(&mut config, |de| de.operator = url("https://example.org/impressum"));

    let german = resolve(&config, "de").expect("resolve");
    assert_eq!(german.footer.imprint.as_deref(), Some("https://example.org/impressum"));
    let privacy = german.privacy.expect("privacy").html;
    assert!(privacy.contains(r#"href="https://example.org/impressum""#), "{privacy}");
    assert!(german.imprint.is_none());

    let english = resolved(&config);
    assert_eq!(english.footer.imprint.as_deref(), Some("https://example.org/imprint"));
    assert!(english.imprint.is_none());
}

/// D5: the imprint's rules apply to the sections in effect for a language,
/// and the error names the keys that clash.
#[test]
fn the_imprint_rules_apply_to_the_sections_of_each_language() {
    let mut config = with_operator();
    config.legal.hosting = text("Hoster AG");
    with_german(&mut config, |de| de.operator = url("https://example.org/impressum"));
    assert!(resolve(&config, "en").is_ok());
    let error = resolve(&config, "de").expect_err("a url with hosting accepted").to_string();
    assert!(error.contains("legal.hosting needs legal.de.operator as text or file"), "{error}");

    // The keys of a language section in its own errors.
    let mut config = with_operator();
    with_german(&mut config, |de| {
        de.terms = Some(LegalSection {
            text: Some("a".to_owned()),
            url: Some("https://example.org/agb".to_owned()),
            ..LegalSection::default()
        })
    });
    let error = resolve(&config, "de").expect_err("text and url accepted").to_string();
    assert!(error.contains("legal.de.terms: set one of text, file and url"), "{error}");
}

#[test]
fn durations_read_naturally() {
    use super::defaults::Facts;

    let mut config = with_operator();
    config.purge.retrieval_timestamp_granularity = crate::config::units::HumanDuration::parse("1h").expect("duration");
    config.purge.inactive_after = crate::config::units::HumanDuration::parse("1d").expect("duration");
    let facts = Facts::from_config(&config, "/en", Some("/en/imprint".to_owned()), false);

    let privacy = defaults::privacy(&facts, "en").join(None);
    assert!(privacy.contains("rounded to 1 hour."), "{privacy}");
    assert!(privacy.contains("1 day after its last retrieval"), "{privacy}");
}

/// AC1: a language's sections follow the same rules as their `[legal.*]`
/// counterparts: one of text/file/url, `extra` only on built-in texts, size
/// and URL limits; the error names the language's key.
#[test]
fn language_sections_follow_the_rules_of_the_default_sections() {
    type Adjust = Box<dyn Fn(&mut LegalTexts)>;
    let cases: Vec<(&str, &str, Adjust)> = vec![
        (
            "text and url",
            "legal.de.privacy",
            Box::new(|de| {
                de.privacy = Some(LegalSection {
                    text: Some("a".to_owned()),
                    url: Some("https://example.org/datenschutz".to_owned()),
                    ..LegalSection::default()
                })
            }),
        ),
        (
            "extra on an own text",
            "legal.de.terms",
            Box::new(|de| {
                de.terms = Some(LegalSection {
                    text: Some("a".to_owned()),
                    extra: Some("b".to_owned()),
                    ..LegalSection::default()
                })
            }),
        ),
        (
            "extra on the operator",
            "legal.de.operator",
            Box::new(|de| {
                de.operator = Some(LegalSection {
                    text: Some("a".to_owned()),
                    extra: Some("b".to_owned()),
                    ..LegalSection::default()
                })
            }),
        ),
        (
            "too long",
            "legal.de.operator",
            Box::new(|de| de.operator = text(&"a".repeat(MAX_TEXT_BYTES + 1))),
        ),
        (
            "too long extra",
            "legal.de.privacy",
            Box::new(|de| {
                de.privacy = Some(LegalSection {
                    extra: Some("a".repeat(MAX_TEXT_BYTES + 1)),
                    ..LegalSection::default()
                })
            }),
        ),
        (
            "script url",
            "legal.de.privacy",
            Box::new(|de| de.privacy = url("javascript:alert(1)")),
        ),
        (
            "protocol-relative url",
            "legal.de.terms",
            Box::new(|de| de.terms = url("//evil.example/agb")),
        ),
        (
            "too long url",
            "legal.de.operator",
            Box::new(|de| de.operator = url(&format!("https://example.org/{}", "a".repeat(MAX_URL_BYTES)))),
        ),
        ("blank text", "legal.de.operator", Box::new(|de| de.operator = text("  \n"))),
        (
            "hosting as url",
            "legal.de.hosting",
            Box::new(|de| de.hosting = url("https://hoster.example")),
        ),
    ];

    for (name, key, adjust) in cases {
        let mut config = with_operator();
        config.legal.hosting = text("Hoster AG");
        with_german(&mut config, |de| adjust(de));
        let error = match resolve(&config, "de") {
            Ok(_) => panic!("{name} accepted in [legal.de]"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains(key), "{name}: {key} not named: {error}");
    }

    // The same values within the limits are accepted.
    let mut config = with_operator();
    with_german(&mut config, |de| {
        de.operator = text(&"a".repeat(MAX_TEXT_BYTES));
        de.privacy = url("https://example.org/datenschutz");
        de.terms = Some(LegalSection {
            extra: Some("Zusatz.".to_owned()),
            ..LegalSection::default()
        });
    });
    assert!(resolve(&config, "de").is_ok());
}
