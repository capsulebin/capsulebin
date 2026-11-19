//! Tests for resolving `[legal]`: which pages exist, what the footer links to,
//! which combinations are refused, and what the built-in texts state.

use super::*;
use crate::config::{Config, LegalSection};

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

fn resolved(config: &Config) -> LegalPages {
    resolve(config).expect("legal configuration must resolve")
}

fn refused(config: &Config) -> String {
    match resolve(config) {
        Ok(_) => panic!("configuration was accepted"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn nothing_configured_means_no_pages_and_no_links() {
    let pages = resolved(&Config::default());

    assert_eq!(pages.footer, "");
    assert!(pages.imprint.is_none() && pages.privacy.is_none() && pages.terms.is_none());
}

#[test]
fn an_operator_brings_the_imprint_and_both_built_in_texts() {
    let pages = resolved(&with_operator());

    assert_eq!(
        pages.footer,
        r#"<a href="/imprint">Imprint</a><a href="/privacy">Privacy</a><a href="/terms">Terms</a>"#
    );
    let imprint = pages.imprint.expect("imprint");
    assert_eq!(imprint.title, "Imprint");
    assert!(imprint.html.contains("Example GmbH<br />"), "{}", imprint.html);
    assert!(pages.privacy.expect("privacy").html.contains("<h2>Controller</h2>"));
    assert!(pages.terms.expect("terms").html.contains("<h2>What you may not share</h2>"));
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
    assert!(pages.footer.contains(r#"<a href="https://operator.example/imprint">Imprint</a>"#));
    assert!(pages.footer.contains(r#"<a href="https://operator.example/privacy">Privacy</a>"#));
    // The built-in terms point to the controller and the privacy notice where they are.
    let terms = pages.terms.expect("terms").html;
    assert!(terms.contains(r#"href="https://operator.example/privacy""#), "{terms}");
}

#[test]
fn privacy_and_terms_of_their_own_need_no_operator() {
    let mut config = Config::default();
    config.legal.privacy = url("https://operator.example/privacy");
    let pages = resolved(&config);

    assert_eq!(pages.footer, r#"<a href="https://operator.example/privacy">Privacy</a>"#);
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
        assert!(resolve(&config).is_err(), "{name} accepted");
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
        assert!(resolve(&config).is_err(), "{href:?} accepted");
    }

    for href in [
        "https://operator.example/privacy",
        "HTTP://operator.example/p?x=1#top",
        "/privacy-notice",
    ] {
        let mut config = Config::default();
        config.legal.privacy = url(href);
        assert!(resolve(&config).is_ok(), "{href:?} refused");
    }
}

#[test]
fn durations_read_naturally() {
    use super::defaults::Facts;

    let mut config = with_operator();
    config.purge.retrieval_timestamp_granularity = crate::config::units::HumanDuration::parse("1h").expect("duration");
    config.purge.inactive_after = crate::config::units::HumanDuration::parse("1d").expect("duration");
    let facts = Facts::from_config(&config, Some("/imprint".to_owned()), false);

    let privacy = defaults::privacy(&facts, None);
    assert!(privacy.contains("rounded to 1 hour."), "{privacy}");
    assert!(privacy.contains("1 day after its last retrieval"), "{privacy}");
}
