use super::{BuiltIn, Facts, privacy, terms};

const DAY: u64 = 86_400;

fn facts(flags: u8) -> Facts {
    let on = |bit: u8| flags & (1 << bit) != 0;
    Facts {
        imprint_href: Some("/de/imprint".to_owned()),
        privacy_href: "/de/privacy".to_owned(),
        report_href: "/de/report".to_owned(),
        hosting: on(0),
        client_ip_limits: on(1),
        purge: on(2),
        never_retrieved_after: 30 * DAY,
        inactive_after: 90 * DAY,
        maximum_expiry: 30 * DAY,
        allow_permanent: on(3),
        retrieval_granularity: DAY,
        reports: on(4),
        report_retention: 180 * DAY,
    }
}

/// Every combination of the flags the texts depend on.
fn every_facts() -> impl Iterator<Item = Facts> {
    (0..32).map(facts)
}

const EXTRA: &str = "\n## Backups\n\nSee [backups](https://operator.example/backups).\n";

fn headings(text: &str) -> usize {
    text.lines().filter(|line| line.starts_with("## ")).count()
}

/// The targets of Markdown links, in order.
fn link_targets(text: &str) -> Vec<&str> {
    text.split("](")
        .skip(1)
        .filter_map(|rest| rest.split_once(')').map(|(target, _)| target))
        .collect()
}

/// The words of `text`, split at everything that is not a letter or digit.
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).collect()
}

/// Whether `word` occurs as a whole word that does not start a sentence,
/// heading or list item.
fn occurs_mid_sentence(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = &text[..at];
        let after = &text[at + word.len()..];
        let whole = !before.ends_with(char::is_alphanumeric) && !after.starts_with(char::is_alphanumeric);
        let starts = match before.trim_end().chars().last() {
            None => true,
            Some(last) => ".!?:#-\n".contains(last),
        };
        whole && !starts
    })
}

#[test]
fn every_language_has_the_same_structure_and_links() {
    for f in every_facts() {
        for (built_in, last) in [(privacy as fn(&Facts, &str) -> BuiltIn, "Deine Rechte"), (terms, "Änderungen")] {
            let en = built_in(&f, "en");
            let de = built_in(&f, "de");
            assert!(de.after_extra.starts_with(&format!("## {last}\n")), "{f:?}: {}", de.after_extra);
            assert_eq!(headings(&de.after_extra), 1, "{f:?}");
            assert_eq!(headings(&en.before_extra), headings(&de.before_extra), "{f:?}: {}", de.before_extra);
            assert_eq!(headings(&en.after_extra), headings(&de.after_extra), "{f:?}");
            for extra in [None, Some(EXTRA)] {
                let (en, de) = (en.join(extra), de.join(extra));
                assert_eq!(headings(&en), headings(&de), "{f:?}, {extra:?}");
                assert_eq!(link_targets(&en), link_targets(&de), "{f:?}, {extra:?}");
            }
        }
    }
}

#[test]
fn the_extra_goes_before_the_last_section() {
    let f = facts(0b1_1111);
    for (text, last) in [
        (privacy(&f, "en"), "## Your rights"),
        (privacy(&f, "de"), "## Deine Rechte"),
        (terms(&f, "en"), "## Changes"),
        (terms(&f, "de"), "## Änderungen"),
    ] {
        let joined = text.join(Some(EXTRA));
        let extra = joined.find("## Backups").expect("extra");
        let section = joined.find(last).expect("last section");
        assert!(extra < section, "{joined}");
        assert_eq!(joined, format!("{}\n\n{}\n\n{}", text.before_extra, EXTRA.trim(), text.after_extra));
        assert_eq!(text.join(None), format!("{}\n\n{}", text.before_extra, text.after_extra));
    }
}

#[test]
fn an_unknown_language_gets_english() {
    let f = facts(0b1_1111);
    assert_eq!(privacy(&f, "fr"), privacy(&f, "en"));
    assert_eq!(terms(&f, "fr"), terms(&f, "en"));
}

#[test]
fn german_texts_contain_no_english_and_no_formal_address() {
    let mut f = facts(0b1_1111);
    f.retrieval_granularity = 3_600;
    f.maximum_expiry = 30 * DAY;
    f.never_retrieved_after = 90 * 60;
    f.inactive_after = DAY;
    f.report_retention = 45;
    let privacy_text = privacy(&f, "de").join(None);
    for expected in [
        "gerundet auf 1 Stunde.",
        "spätestens 30 Tage nach",
        "- 90 Minuten nach ihrer Erstellung",
        "sonst 1 Tag nach ihrem letzten Abruf",
        "nach 45 Sekunden gelöscht",
        "DSGVO",
        "Art. 6 Abs. 1 lit. b DSGVO",
        "`capsulebin.theme`",
        "`capsulebin.language`",
    ] {
        assert!(privacy_text.contains(expected), "{expected:?} missing: {privacy_text}");
    }

    f.report_retention = 30 * DAY;
    let privacy_text = privacy(&f, "de").join(None);
    assert!(privacy_text.contains("nach 30 Tagen gelöscht"), "{privacy_text}");

    for flags in 0..32 {
        let mut f = facts(flags);
        f.retrieval_granularity = 45;
        f.never_retrieved_after = 90 * 60;
        f.inactive_after = 3_600;
        for text in [privacy(&f, "de").join(None), terms(&f, "de").join(None)] {
            let words = words(&text);
            for english in ["day", "days", "hour", "hours", "minute", "minutes", "second", "seconds", "GDPR"] {
                assert!(!words.contains(&english), "{english:?} in: {text}");
            }
            assert!(!text.contains("Legal basis"), "{text}");
            for formal in ["Ihr", "Ihre", "Ihren", "Ihrem", "Ihrer", "Ihres", "Ihnen"] {
                assert!(!words.contains(&formal), "{formal:?} in: {text}");
            }
            assert!(!occurs_mid_sentence(&text, "Sie"), "formal \"Sie\" in: {text}");
        }
    }
}

#[test]
fn the_formal_address_check_finds_it_mid_sentence_only() {
    assert!(occurs_mid_sentence("Wenn Sie melden", "Sie"));
    assert!(!occurs_mid_sentence("Sie wird gelöscht. Sie bleibt", "Sie"));
    assert!(!occurs_mid_sentence("Wenn Siegel", "Sie"));
}
