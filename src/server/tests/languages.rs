#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The known interface languages exist in three places - the server's list, the
//! core list and the catalogues of the web client - because each is built on its
//! own. This test keeps the server's list and the catalogue files equal; a
//! language in one but not the other fails it, whichever way round.

use std::collections::BTreeSet;
use std::path::Path;

use capsulebin_server::config::KNOWN_LANGUAGES;

#[test]
fn known_languages_match_the_web_catalogues() {
    let catalogues = Path::new(env!("CARGO_MANIFEST_DIR")).join("web/src/i18n");
    let mut files = BTreeSet::new();
    for entry in std::fs::read_dir(&catalogues).expect("the catalogue directory") {
        let path = entry.expect("a catalogue directory entry").path();
        if path.extension().is_some_and(|extension| extension == "json")
            && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        {
            files.insert(stem.to_owned());
        }
    }

    let known: BTreeSet<String> = KNOWN_LANGUAGES.iter().map(|code| (*code).to_owned()).collect();
    assert_eq!(known.len(), KNOWN_LANGUAGES.len(), "KNOWN_LANGUAGES lists a code twice");
    assert_eq!(
        known,
        files,
        "KNOWN_LANGUAGES and the catalogues in {} differ",
        catalogues.display()
    );
}
