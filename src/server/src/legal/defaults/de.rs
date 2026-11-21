//! The built-in texts in German, addressing the reader with "du" as the
//! interface does.

use super::{BuiltIn, Facts, Unit, unit};

/// The grammatical case a duration stands in; only the plural of "Tag" tells
/// them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    /// Also the accusative: "spätestens 30 Tage nach", "gerundet auf 30 Tage".
    Nominative,
    /// After "nach", "von", "mit": "nach 30 Tagen".
    Dative,
}

/// A duration in the largest whole unit: "90 Tage", "1 Stunde"; in the
/// dative "nach 90 Tagen".
fn duration(seconds: u64, case: Case) -> String {
    let (count, unit) = unit(seconds);
    let (singular, plural) = match unit {
        Unit::Day => (
            "Tag",
            match case {
                Case::Nominative => "Tage",
                Case::Dative => "Tagen",
            },
        ),
        Unit::Hour => ("Stunde", "Stunden"),
        Unit::Minute => ("Minute", "Minuten"),
        Unit::Second => ("Sekunde", "Sekunden"),
    };
    format!("{count} {}", if count == 1 { singular } else { plural })
}

pub(super) fn privacy(f: &Facts) -> BuiltIn {
    let d = |seconds| duration(seconds, Case::Nominative);
    let mut text = String::new();

    text.push_str(&format!(
        "## Verantwortlicher\n\n\
         Verantwortlich für diesen Dienst ist der im [Impressum]({}) genannte Betreiber.",
        f.imprint()
    ));
    if f.hosting {
        text.push_str(
            " Der Dienst wird bei dem dort genannten Hostinganbieter betrieben, der Daten im Auftrag \
             des Betreibers verarbeitet (Art. 28 DSGVO).",
        );
    }

    text.push_str(
        "\n\n## Was dieser Dienst nicht sehen kann\n\n\
         Kapseln werden auf deinem Gerät verschlüsselt (in deinem Browser oder im \
         Kommandozeilen-Client), bevor sie hochgeladen werden. Der Schlüssel ist Teil des Links, und \
         zwar des Teils nach `#`, den Browser nicht an den Server senden. Wir können deshalb weder den \
         Inhalt einer Kapsel noch ihren Titel oder die Namen der Dateien darin lesen und sie an \
         niemanden weitergeben. Auch ein Passwort, das du für eine Kapsel festlegst, verlässt dein \
         Gerät nicht.\n\n\
         ## Kapseln\n\n\
         Wir speichern die verschlüsselte Kapsel zusammen mit den technischen Daten, die nötig sind, \
         um sie auszuliefern und zu löschen: eine ID, Prüfwerte (Hashes) ihrer Zugangsgeheimnisse, \
         ihre Größe, wann sie erstellt wurde und wann sie abläuft, ob sie nach dem ersten Abruf \
         gelöscht wird, und wann sie zuletzt abgerufen wurde",
    );
    text.push_str(&format!(
        ", gerundet auf {}. Rechtsgrundlage: Art. 6 Abs. 1 lit. b DSGVO, die Bereitstellung des \
         Dienstes, den du nutzt, und Art. 6 Abs. 1 lit. f DSGVO, unser berechtigtes Interesse an \
         seinem sicheren Betrieb.\n\n\
         Eine Kapsel wird gelöscht\n\n",
        d(f.retrieval_granularity)
    ));
    if f.allow_permanent {
        text.push_str(&format!(
            "- wenn sie abläuft, spätestens {} nach ihrer Erstellung, sofern sie nicht ohne Ablauf \
             erstellt wurde,\n",
            d(f.maximum_expiry)
        ));
    } else {
        text.push_str(&format!(
            "- wenn sie abläuft, spätestens {} nach ihrer Erstellung,\n",
            d(f.maximum_expiry)
        ));
    }
    text.push_str(
        "- wenn jemand sie mit ihrem Löschlink löscht,\n\
         - direkt nach ihrem ersten Abruf, wenn das bei der Erstellung gewählt wurde,\n",
    );
    if f.purge {
        text.push_str(&format!(
            "- {} nach ihrer Erstellung, wenn sie nie abgerufen wurde, und sonst {} nach ihrem letzten \
             Abruf.\n",
            d(f.never_retrieved_after),
            d(f.inactive_after)
        ));
    }
    text.push_str(
        "\nDer verschlüsselte Inhalt wird in der Datenbank überschrieben, wenn eine Kapsel gelöscht \
         wird. Die technischen Daten der Kapsel, ohne Inhalt, bleiben erhalten, bis eine der Regeln \
         oben sie entfernt. Ist eine Kapsel entfernt, bleibt nur ihre Kennung erhalten, damit sie nie \
         für eine andere Kapsel verwendet werden kann.\n\n\
         ## IP-Adressen und Server-Logs\n\n",
    );
    if f.client_ip_limits {
        text.push_str(
            "Um die Zahl der Anfragen je Client zu begrenzen, verarbeitet der Server deine IP-Adresse \
             in seinem Arbeitsspeicher. Die Adresse wird weder in die Datenbank noch in die Logs des \
             Servers geschrieben und wird verworfen, sobald die Begrenzung sie nicht mehr braucht, \
             spätestens beim Neustart des Servers. Rechtsgrundlage: Art. 6 Abs. 1 lit. f DSGVO, unser \
             berechtigtes Interesse am Schutz des Dienstes vor Missbrauch.",
        );
    } else {
        text.push_str("Die Serveranwendung speichert keine IP-Adressen.");
    }
    text.push_str(
        " Das Log der Serveranwendung verzeichnet für jede Anfrage die Methode, die Adresse ohne alle \
         Kennungen, das Ergebnis und die Dauer – keine IP-Adressen und keine Inhalte.",
    );

    if f.reports {
        text.push_str(&format!(
            "\n\n## Meldungen\n\n\
             Wenn du eine Kapsel meldest, speichern wir ihre ID, den von dir gewählten Grund, deine \
             Beschreibung und, falls du sie angibst, deine Kontaktdaten, um die Meldung zu prüfen und \
             darauf zu reagieren. Rechtsgrundlage: Art. 6 Abs. 1 lit. f DSGVO, unser berechtigtes \
             Interesse daran, den Dienst frei von rechtswidrigen Inhalten zu halten, und Art. 6 Abs. 1 \
             lit. c DSGVO, soweit uns das Gesetz verpflichtet, auf Meldungen zu reagieren. Offene \
             Meldungen werden aufbewahrt, bis sie bearbeitet sind; bearbeitete Meldungen und die \
             Aufzeichnung der ergriffenen Maßnahmen werden nach {} gelöscht.",
            duration(f.report_retention, Case::Dative)
        ));
    }

    text.push_str(
        "\n\n## Cookies und Dritte\n\n\
         Dieser Dienst setzt keine Cookies. In deinem Browser gespeichert werden nur das Farbschema \
         und die Sprache, die du wählst, und zwar in seinem lokalen Speicher (`capsulebin.theme`, \
         `capsulebin.language`); sie werden nicht an uns gesendet, die Sprache ist aber Teil der \
         Adresse jeder Seite, die du öffnest. Die Seiten laden nichts von Dritten: keine Webanalyse, \
         keine externen Schriftarten, kein Tracking. Bilder in Kapseln werden als Links angezeigt und \
         nur geladen, wenn du ihnen folgst.",
    );

    BuiltIn::new(
        text,
        "## Deine Rechte\n\n\
         Du hast das Recht auf Auskunft, Berichtigung, Löschung, Einschränkung der Verarbeitung, \
         Datenübertragbarkeit und Widerspruch (Art. 15 bis 21 DSGVO) sowie das Recht auf Beschwerde \
         bei einer Aufsichtsbehörde (Art. 77 DSGVO). Da wir Kapseln weder lesen können noch wissen, \
         wer sie erstellt hat, können wir eine Kapsel nur anhand ihrer ID finden – dem Teil des Links \
         vor `#`. Um eine Kapsel entfernen zu lassen, lösche sie mit ihrem Löschlink oder schick uns \
         ihre ID; schick nie den vollständigen Link, denn er enthält den Schlüssel.\n",
    )
}

pub(super) fn terms(f: &Facts) -> BuiltIn {
    let mut text = String::from(
        "Mit diesem Dienst teilst du verschlüsselten Text, Code und Dateien über einen Link. Wenn du \
         ihn nutzt, stimmst du diesen Regeln zu.\n\n\
         ## Was du nicht teilen darfst\n\n\
         Nutze den Dienst nicht für rechtswidrige Inhalte oder Zwecke, insbesondere nicht für \
         Inhalte, die Rechte anderer verletzen, sexuellen Missbrauch von Kindern darstellen, zu \
         Gewalt oder Hass aufstacheln oder dazu dienen, Schadsoftware zu verbreiten oder Menschen zu \
         täuschen (Phishing).\n\n\
         ## Meldungen und Moderation\n\n",
    );
    if f.reports {
        text.push_str(&format!(
            "Jede Person kann eine Kapsel über das [Meldeformular]({}) melden. ",
            f.report_href
        ));
    }
    text.push_str(
        &"Da Kapseln verschlüsselt sind, können wir ihren Inhalt nicht prüfen. Auf einen begründeten \
         Hinweis hin sperren oder löschen wir eine Kapsel anhand ihrer ID.\n\n\
         ## Verfügbarkeit\n\n\
         Der Dienst wird bereitgestellt, wie er ist, ohne Gewähr, dass er jederzeit verfügbar ist. \
         Kapseln werden nach den Regeln in der [Datenschutzerklärung]({privacy}) gelöscht und lassen \
         sich danach nicht wiederherstellen; bewahre eine eigene Kopie von allem auf, was du noch \
         brauchst."
            .replace("{privacy}", &f.privacy_href),
    );
    BuiltIn::new(
        text,
        "## Änderungen\n\n\
         Wir können diese Regeln ändern. Es gilt die hier angezeigte Fassung.\n",
    )
}

#[cfg(test)]
mod tests {
    use super::{Case, duration};

    #[test]
    fn durations_take_the_german_unit_and_case() {
        for (seconds, nominative, dative) in [
            (86_400, "1 Tag", "1 Tag"),
            (30 * 86_400, "30 Tage", "30 Tagen"),
            (3_600, "1 Stunde", "1 Stunde"),
            (2 * 3_600, "2 Stunden", "2 Stunden"),
            (60, "1 Minute", "1 Minute"),
            (90 * 60, "90 Minuten", "90 Minuten"),
            (1, "1 Sekunde", "1 Sekunde"),
            (45, "45 Sekunden", "45 Sekunden"),
        ] {
            assert_eq!(duration(seconds, Case::Nominative), nominative);
            assert_eq!(duration(seconds, Case::Dative), dative);
        }
    }
}
