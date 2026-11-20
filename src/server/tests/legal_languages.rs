#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The legal pages in every interface language (package
//! 20261120-legal-languages): `[legal.<lang>.*]`, the fallback per section to
//! `[legal.*]`, the `lang` marking of what is not in the page's language, and
//! the titles from the shell.

mod support;

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use capsulebin_server::config::{Config, LegalSection, LegalTexts};
use support::*;

fn url(href: &str) -> Option<LegalSection> {
    Some(LegalSection {
        url: Some(href.to_owned()),
        ..LegalSection::default()
    })
}

/// Sets `[legal.<language>.*]`.
fn with_language(config: &mut Config, language: &str, adjust: impl FnOnce(&mut LegalTexts)) {
    adjust(config.legal.languages.entry(language.to_owned()).or_default());
}

/// The `<article>` of a legal page, as served.
fn article(page: &str) -> &str {
    let start = page.find("<article").unwrap_or_else(|| panic!("no article: {page}"));
    let end = page.find("</article>").unwrap_or_else(|| panic!("no article end: {page}"));
    &page[start..end + "</article>".len()]
}

/// The values of the `lang` attributes in `html`, in order.
fn langs(html: &str) -> Vec<&str> {
    html.split(" lang=\"").skip(1).filter_map(|rest| rest.split('"').next()).collect()
}

/// AC3: with only `[legal.operator] text`, the German imprint is that text,
/// marked as English as a whole; the German privacy notice and terms are the
/// built-in German texts, unmarked; nothing is marked in the English pages.
#[tokio::test]
async fn ac3_each_section_falls_back_to_the_default_language() {
    let h = harness_with("legal-lang-fallback", |config| {
        config.legal.operator = legal_text("Example GmbH");
    })
    .await;

    let imprint = h.get("/de/imprint").await.text();
    assert!(
        article(&imprint).starts_with(r#"<article lang="en"><p>Example GmbH</p>"#),
        "{imprint}"
    );
    assert_eq!(langs(article(&imprint)), ["en"]);

    for (path, german) in [("/de/privacy", "DSGVO"), ("/de/terms", "Was du nicht teilen darfst")] {
        let page = h.get(path).await.text();
        assert!(article(&page).contains(german), "{path}: {page}");
        assert!(langs(article(&page)).is_empty(), "{path}: {page}");
    }
    for path in ["/imprint", "/privacy", "/terms"] {
        let page = h.get(path).await.text();
        assert!(langs(article(&page)).is_empty(), "{path}: {page}");
    }
}

/// AC4: a block in another language is marked on its own: the English
/// `extra` in the German privacy notice, and the English hosting block below
/// a German operator text.
#[tokio::test]
async fn ac4_blocks_in_another_language_are_marked_on_their_own() {
    let h = harness_with("legal-lang-blocks", |config| {
        config.legal.operator = legal_text("Example GmbH");
        config.legal.hosting = legal_text("Hoster AG");
        config.legal.privacy = Some(LegalSection {
            extra: Some("Our proxy keeps logs for 7 days.".to_owned()),
            ..LegalSection::default()
        });
        with_language(config, "de", |de| de.operator = legal_text("Beispiel GmbH"));
    })
    .await;

    let privacy = h.get("/de/privacy").await.text();
    let privacy = article(&privacy);
    assert!(privacy.starts_with("<article><h2>Verantwortlicher</h2>"), "{privacy}");
    assert_eq!(langs(privacy), ["en"], "{privacy}");
    assert!(
        privacy.contains("<div lang=\"en\"><p>Our proxy keeps logs for 7 days.</p>\n</div>"),
        "{privacy}"
    );

    let imprint = h.get("/de/imprint").await.text();
    assert_eq!(
        article(&imprint),
        "<article><p>Beispiel GmbH</p>\n<div lang=\"en\"><h2>Hosting</h2>\n<p>Hoster AG</p>\n</div></article>"
    );
}

/// AC5: a language section replaces its default section as a whole.
#[tokio::test]
async fn ac5_a_language_section_replaces_the_default_one() {
    let h = harness_with("legal-lang-replace", |config| {
        config.legal.operator = legal_text("Example GmbH");
        config.legal.privacy = legal_text("Our own notice.");
        config.legal.terms = Some(LegalSection {
            extra: Some("English extra.".to_owned()),
            ..LegalSection::default()
        });
        with_language(config, "de", |de| {
            de.privacy = Some(LegalSection::default());
            de.terms = Some(LegalSection {
                extra: Some("Deutscher Zusatz.".to_owned()),
                ..LegalSection::default()
            });
        });
    })
    .await;

    let privacy = h.get("/de/privacy").await.text();
    assert!(privacy.contains("DSGVO") && !privacy.contains("Our own notice."), "{privacy}");
    let terms = h.get("/de/terms").await.text();
    assert!(terms.contains("Deutscher Zusatz.") && !terms.contains("English extra."), "{terms}");
    assert!(h.get("/terms").await.text().contains("English extra."));
}

/// AC6: with German as the default, `[legal.*]` is German: unmarked under
/// `/imprint`, marked `lang="de"` under `/en/imprint`; the English built-in
/// texts are unmarked and link under `/en` (the report page from the terms:
/// the privacy notice has no link to it).
#[tokio::test]
async fn ac6_with_german_as_default_the_default_texts_are_german() {
    let h = harness_with("legal-lang-default-de", |config| {
        config.i18n.default_language = "de".to_owned();
        config.legal.operator = legal_text("Beispiel GmbH");
    })
    .await;

    let german = h.get("/imprint").await.text();
    assert_eq!(article(&german), "<article><p>Beispiel GmbH</p>\n</article>");
    let english = h.get("/en/imprint").await.text();
    assert_eq!(article(&english), "<article lang=\"de\"><p>Beispiel GmbH</p>\n</article>");

    let privacy = h.get("/en/privacy").await.text();
    let privacy = article(&privacy);
    assert!(privacy.contains("<h2>Controller</h2>") && privacy.contains("GDPR"), "{privacy}");
    assert!(langs(privacy).is_empty(), "{privacy}");
    assert!(privacy.contains(r#"href="/en/imprint""#), "{privacy}");
    let terms = h.get("/en/terms").await.text();
    let terms = article(&terms);
    assert!(langs(terms).is_empty() && terms.contains(r#"href="/en/report""#), "{terms}");
}

/// AC7: `url` per language: the footer of each language and the controller of
/// its built-in privacy notice link to that language's imprint; neither is
/// served here.
#[tokio::test]
async fn ac7_a_url_per_language_decides_the_links_of_that_language() {
    let h = harness_with("legal-lang-url", |config| {
        config.legal.operator = url("https://example.org/imprint");
        with_language(config, "de", |de| de.operator = url("https://example.org/impressum"));
    })
    .await;

    for (prefix, href) in [("/de", "https://example.org/impressum"), ("", "https://example.org/imprint")] {
        for page in [
            "/",
            "/p/AAAAAAAAAAAAAAAAAAAAAA",
            "/manage/AAAAAAAAAAAAAAAAAAAAAA",
            "/report",
            "/privacy",
            "/terms",
        ] {
            let path = format!("{prefix}{page}");
            let text = h.get(&path).await.text();
            assert!(text.contains(&format!(r#"<footer><a href="{href}">"#)), "{path}: {text}");
        }
        let privacy = h.get(&format!("{prefix}/privacy")).await.text();
        assert!(article(&privacy).contains(&format!(r#"href="{href}""#)), "{prefix}: {privacy}");

        let unknown = h.get("/no-such-page").await;
        let imprint = h.get(&format!("{prefix}/imprint")).await;
        assert_eq!(imprint.status, StatusCode::NOT_FOUND, "{prefix}/imprint");
        assert_eq!(imprint.body, unknown.body, "{prefix}/imprint");
    }
}

/// AC10: the links in the built-in texts carry the page language's prefix:
/// the privacy notice to the imprint, the terms to the privacy notice and the
/// report page.
#[tokio::test]
async fn ac10_built_in_links_carry_the_prefix_of_the_page() {
    let h = harness_with("legal-lang-links", |config| {
        config.legal.operator = legal_text("Example GmbH");
    })
    .await;

    for prefix in ["/de", ""] {
        let privacy = h.get(&format!("{prefix}/privacy")).await.text();
        let terms = h.get(&format!("{prefix}/terms")).await.text();
        let (privacy, terms) = (article(&privacy), article(&terms));
        assert!(privacy.contains(&format!(r#"href="{prefix}/imprint""#)), "{privacy}");
        assert!(terms.contains(&format!(r#"href="{prefix}/privacy""#)), "{terms}");
        assert!(terms.contains(&format!(r#"href="{prefix}/report""#)), "{terms}");
    }
}

/// AC11: `<title>`, header and `<h1>` are the footer label of the page in the
/// page's language, the `<h1>` outside the article that may carry another
/// `lang`.
#[tokio::test]
async fn ac11_titles_are_the_labels_of_the_page_language() {
    let h = harness_with("legal-lang-titles", |config| {
        config.legal.operator = legal_text("Example GmbH");
    })
    .await;

    for (path, label) in [
        ("/de/imprint", "Impressum"),
        ("/de/privacy", "Datenschutz"),
        ("/de/terms", "Nutzungsbedingungen"),
        ("/imprint", "Imprint"),
        ("/privacy", "Privacy"),
        ("/terms", "Terms"),
    ] {
        let page = h.get(path).await.text();
        assert!(page.contains(&format!("<title>{label} · CapsuleBin</title>")), "{path}: {page}");
        assert!(page.contains(&format!("<header>{label}</header>")), "{path}: {page}");
        assert!(
            page.contains(&format!(r#"<div class="cb-prose cb-legal"><h1>{label}</h1><article"#)),
            "{path}: {page}"
        );
        assert!(!article(&page).contains("<h1>"), "{path}: {page}");
    }

    let source = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/legal.rs")).unwrap();
    for title in [r#""Imprint""#, r#""Privacy notice""#, r#""Terms of use""#] {
        assert!(!source.contains(title), "legal.rs still holds the title {title}");
    }
}

/// A configuration file with shells from `asset_dir` (so neither `check` nor
/// `serve` depends on the embedded web build), `legal` appended.
fn config_file(dir: &Path, name: &str, legal: &str) -> PathBuf {
    let assets = dir.join("assets");
    write_shells(&assets);
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n{legal}",
            assets.display(),
            dir.join(format!("{name}.db")).display()
        ),
    )
    .unwrap();
    path
}

fn check(config: &Path, env: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_capsulebin"));
    command.arg("--config").arg(config).arg("check").envs(env.iter().copied());
    command.output().unwrap()
}

/// AC2: `[legal.<default language>]`, a table that is not a known language,
/// an unknown key inside a language table, and a translated operator or
/// hosting provider without the default language's stop `check`, naming the
/// key; so does an operator `url` of a language with a hosting provider in
/// effect (D5).
#[test]
fn ac2_check_rejects_language_tables_that_cannot_be_meant() {
    let dir = tempdir("legal-lang-check");
    let operator = "[legal.operator]\ntext = \"Example GmbH\"\n";

    let ok = config_file(
        &dir,
        "ok.toml",
        &format!("{operator}[legal.de.operator]\ntext = \"Beispiel GmbH\"\n[legal.de.privacy]\n"),
    );
    let output = check(&ok, &[]);
    assert!(output.status.success(), "rejected: {}", String::from_utf8_lossy(&output.stderr));

    for (index, (legal, env, named)) in [
        (
            format!("{operator}[legal.en.operator]\ntext = \"x\"\n"),
            &[][..],
            &["legal.en", "[legal.*]"][..],
        ),
        (
            "[i18n]\ndefault_language = \"de\"\n[legal.operator]\ntext = \"x\"\n[legal.de.terms]\ntext = \"x\"\n".to_owned(),
            &[],
            &["legal.de", "[legal.*]"],
        ),
        (format!("{operator}[legal.fr.operator]\ntext = \"x\"\n"), &[], &["legal.fr"]),
        (
            operator.to_owned(),
            &[("CAPSULEBIN__LEGAL__FR__OPERATOR__TEXT", "x")],
            &["legal.fr"],
        ),
        (format!("{operator}[legal.de]\nimprint = \"x\"\n"), &[], &["legal.de.imprint"]),
        (
            format!("{operator}[legal.de.operator]\nimprint = \"x\"\n"),
            &[],
            &["legal.de.operator", "imprint"],
        ),
        (
            "[legal.de.operator]\ntext = \"x\"\n".to_owned(),
            &[],
            &["legal.de.operator", "legal.operator"],
        ),
        (
            format!("{operator}[legal.de.hosting]\ntext = \"x\"\n"),
            &[],
            &["legal.de.hosting", "legal.hosting"],
        ),
        (
            format!("{operator}[legal.hosting]\ntext = \"x\"\n[legal.de.operator]\nurl = \"https://example.org/impressum\"\n"),
            &[],
            &["legal.hosting", "legal.de.operator"],
        ),
        (
            format!("{operator}[legal.de.terms]\ntext = \"x\"\nextra = \"y\"\n"),
            &[],
            &["legal.de.terms.extra"],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let config = config_file(&dir, &format!("bad-{index}.toml"), &legal);
        let output = check(&config, env);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{legal} {env:?} accepted");
        for key in named {
            assert!(stderr.contains(key), "{legal} {env:?}: {key} not named: {stderr}");
        }
    }
}

/// Kills the server when the test ends, also on a failed assertion.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `GET path` over a real socket, the whole answer as text.
fn get(port: u16, path: &str) -> Option<String> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    Some(answer)
}

/// `serve` with `config` and `env`, and the answer to `GET /de/imprint`.
fn served_german_imprint(config: &Path, env: &[(&str, &str)]) -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let bind = format!("127.0.0.1:{port}");
    let mut command = Command::new(env!("CARGO_BIN_EXE_capsulebin"));
    command
        .arg("--config")
        .arg(config)
        .arg("serve")
        .env("CAPSULEBIN__SERVER__BIND", &bind)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _server = Server(command.spawn().unwrap());

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(answer) = get(port, "/de/imprint") {
            return answer;
        }
        assert!(Instant::now() < deadline, "the server did not answer on {bind}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// AC1: `[legal.de.operator]` from the file and from the environment is the
/// German imprint (default `en`), unmarked.
#[test]
fn ac1_a_german_imprint_from_the_file_and_from_the_environment() {
    let dir = tempdir("legal-lang-serve");
    let operator = "[legal.operator]\ntext = \"Example GmbH\"\n";

    let file = config_file(
        &dir,
        "file.toml",
        &format!("{operator}[legal.de.operator]\ntext = \"Beispiel aus der Datei\"\n"),
    );
    let answer = served_german_imprint(&file, &[]);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(
        answer.contains("<h1>Impressum</h1><article><p>Beispiel aus der Datei</p>"),
        "{answer}"
    );

    let env = config_file(&dir, "env.toml", operator);
    let answer = served_german_imprint(&env, &[("CAPSULEBIN__LEGAL__DE__OPERATOR__TEXT", "Beispiel aus der Umgebung")]);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(
        answer.contains("<h1>Impressum</h1><article><p>Beispiel aus der Umgebung</p>"),
        "{answer}"
    );
}

/// AC1: `file` of a language section from the environment
/// (`CAPSULEBIN__LEGAL__DE__OPERATOR__FILE`), and a URL limit of a language
/// section set from the environment stops `check`, naming the key.
#[test]
fn ac1_language_sections_from_the_environment_follow_the_same_rules() {
    let dir = tempdir("legal-lang-env-file");
    let operator = "[legal.operator]\ntext = \"Example GmbH\"\n";
    let markdown = dir.join("impressum.md");
    std::fs::write(&markdown, "Beispiel aus **der Datei** der Umgebung").unwrap();

    let config = config_file(&dir, "env-file.toml", operator);
    let file = markdown.to_str().unwrap();
    let answer = served_german_imprint(&config, &[("CAPSULEBIN__LEGAL__DE__OPERATOR__FILE", file)]);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(
        answer.contains("<h1>Impressum</h1><article><p>Beispiel aus <strong>der Datei</strong> der Umgebung</p>"),
        "{answer}"
    );

    for (key, value, named) in [
        ("CAPSULEBIN__LEGAL__DE__PRIVACY__URL", "javascript:alert(1)", "legal.de.privacy"),
        ("CAPSULEBIN__LEGAL__DE__OPERATOR__EXTRA", "x", "legal.de.operator"),
        ("CAPSULEBIN__LEGAL__DE__OPERATOR__IMPRINT", "x", "legal.de.operator"),
    ] {
        let output = check(&config, &[(key, value)]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{key} accepted");
        assert!(stderr.contains(named), "{key}: {named} not named: {stderr}");
    }
}
