//! The built-in privacy notice and terms of use.
//!
//! English only, in the pages of every language; the article says so in a shell
//! of another language (`lang="en"`).
//!
//! What CapsuleBin does with data is fixed by the software and its
//! configuration, so these texts state it concretely - the retention periods
//! and whether client addresses are processed come from the running
//! configuration, not from a template the operator has to keep in sync. What
//! the software cannot know (the operator's identity, a hosting provider, the
//! reverse proxy's logs, backups) comes from `[legal]`: the operator and the
//! hosting provider are named in the imprint, everything else goes into
//! `extra`.
//!
//! The texts are a careful default, not legal advice; the operator stays
//! responsible for what the instance publishes (docs/privacy-notice.md).

use crate::config::Config;

/// What the texts state about the running instance.
#[derive(Debug, Clone)]
pub struct Facts {
    /// Where the imprint is; `None` when there is no operator.
    pub imprint_href: Option<String>,
    /// Where the privacy notice is, for the terms to refer to.
    pub privacy_href: String,
    /// Where the report form is, for the terms to refer to.
    pub report_href: String,
    pub hosting: bool,
    /// Whether any limit keyed by the client address is on.
    pub client_ip_limits: bool,
    pub purge: bool,
    pub never_retrieved_after: u64,
    pub inactive_after: u64,
    pub maximum_expiry: u64,
    pub allow_permanent: bool,
    pub retrieval_granularity: u64,
    pub reports: bool,
    pub report_retention: u64,
}

impl Facts {
    /// `prefix` is the language prefix of the shell the texts are shown in
    /// (`/de`, empty for the default language), for the links to pages served
    /// here.
    pub fn from_config(config: &Config, prefix: &str, imprint_href: Option<String>, hosting: bool) -> Self {
        Self {
            imprint_href,
            privacy_href: format!("{prefix}/privacy"),
            report_href: format!("{prefix}/report"),
            hosting,
            client_ip_limits: config.rate_limits.uses_client_ip(),
            purge: config.purge.enabled,
            never_retrieved_after: config.purge.never_retrieved_after.as_secs(),
            inactive_after: config.purge.inactive_after.as_secs(),
            maximum_expiry: config.pastes.maximum_expiry.as_secs(),
            allow_permanent: config.pastes.allow_permanent,
            retrieval_granularity: config.purge.retrieval_timestamp_granularity.as_secs(),
            reports: config.abuse.enabled,
            report_retention: config.abuse.report_retention.as_secs(),
        }
    }

    fn imprint(&self) -> &str {
        self.imprint_href.as_deref().unwrap_or("/imprint")
    }
}

/// A duration in the largest whole unit: "90 days", "1 hour".
fn duration(seconds: u64) -> String {
    let (count, singular, plural) = if seconds.is_multiple_of(86_400) {
        (seconds / 86_400, "day", "days")
    } else if seconds.is_multiple_of(3_600) {
        (seconds / 3_600, "hour", "hours")
    } else if seconds.is_multiple_of(60) {
        (seconds / 60, "minute", "minutes")
    } else {
        (seconds, "second", "seconds")
    };
    format!("{count} {}", if count == 1 { singular } else { plural })
}

pub fn privacy(f: &Facts, extra: Option<&str>) -> String {
    let d = duration;
    let mut text = String::new();

    text.push_str(&format!(
        "## Controller\n\n\
         The controller for this service is the operator named in the [imprint]({}).",
        f.imprint()
    ));
    if f.hosting {
        text.push_str(
            " The service is hosted by the hosting provider named there, which processes data on the \
             operator's behalf (Art. 28 GDPR).",
        );
    }

    text.push_str(
        "\n\n## What this service cannot see\n\n\
         Capsules are encrypted on your device (in your browser or in the command line client) \
         before they are uploaded. The key is part of the link, in the part after `#`, which browsers \
         do not send to the server. We therefore cannot read the content of a capsule, its title or \
         the names of the files in it, and cannot pass them on to anyone. A password you set for a \
         capsule does not leave your device either.\n\n\
         ## Capsules\n\n\
         We store the encrypted capsule together with the technical data needed to deliver and delete \
         it: an identifier, check values (hashes) of its access secrets, its size, when it was created \
         and when it expires, whether it is deleted after the first retrieval, and when it was last \
         retrieved",
    );
    text.push_str(&format!(
        ", rounded to {}. Legal basis: Art. 6(1)(b) GDPR, providing the service you use, and \
         Art. 6(1)(f) GDPR, our legitimate interest in operating it securely.\n\n\
         A capsule is deleted\n\n",
        d(f.retrieval_granularity)
    ));
    if f.allow_permanent {
        text.push_str(&format!(
            "- when it expires, at the latest {} after it was created, unless it was created without an \
             expiry,\n",
            d(f.maximum_expiry)
        ));
    } else {
        text.push_str(&format!(
            "- when it expires, at the latest {} after it was created,\n",
            d(f.maximum_expiry)
        ));
    }
    text.push_str(
        "- when someone deletes it with its management link,\n\
         - right after its first retrieval, if that was chosen when it was created,\n",
    );
    if f.purge {
        text.push_str(&format!(
            "- {} after it was created if it was never retrieved, and {} after its last retrieval \
             otherwise.\n",
            d(f.never_retrieved_after),
            d(f.inactive_after)
        ));
    }
    text.push_str(
        "\nThe encrypted content is overwritten in the database when a capsule is deleted. Its technical \
         data, without content, remains until one of the rules above removes it.\n\n\
         ## IP addresses and server logs\n\n",
    );
    if f.client_ip_limits {
        text.push_str(
            "To limit the number of requests per client, the server processes your IP address in its \
             memory. It is not written to the database or to the server's logs, and is discarded once the \
             limit no longer needs it, at the latest when the server restarts. Legal basis: Art. 6(1)(f) \
             GDPR, our legitimate interest in protecting the service against abuse.",
        );
    } else {
        text.push_str("The server application does not store IP addresses.");
    }
    text.push_str(
        " Its log records, for each request, the method, the address with all identifiers removed, \
         the result and the duration - no IP addresses and no content.",
    );

    if f.reports {
        text.push_str(&format!(
            "\n\n## Reports\n\n\
             If you report a capsule, we store its identifier, the reason you choose, your description \
             and, if you give them, your contact details, to review the report and act on it. Legal \
             basis: Art. 6(1)(f) GDPR, our legitimate interest in keeping the service free of unlawful \
             content, and Art. 6(1)(c) GDPR where the law obliges us to act on reports. Open reports are \
             kept until they are handled; handled reports and the record of the actions taken are \
             deleted after {}.",
            d(f.report_retention)
        ));
    }

    text.push_str(
        "\n\n## Cookies and third parties\n\n\
         This service sets no cookies. The only things stored in your browser are the colour theme \
         and the language you choose, in its local storage (`capsulebin.theme`, \
         `capsulebin.language`); they are not sent to us, but the language is part of the \
         address of each page you open. Pages load nothing from third parties: \
         no analytics, no external fonts, no tracking. Images in capsules are shown as links and only \
         loaded if you follow them.",
    );

    if let Some(extra) = extra {
        text.push_str("\n\n");
        text.push_str(extra.trim());
    }

    text.push_str(
        "\n\n## Your rights\n\n\
         You have the right of access, to rectification, to erasure, to restriction of processing, to \
         data portability and to object (Art. 15 to 21 GDPR), and the right to lodge a complaint with \
         a supervisory authority (Art. 77 GDPR). As we can neither read capsules nor tell who created \
         them, we can find a capsule only by its identifier - the part of the link before `#`. To have \
         a capsule removed, delete it with its management link, or send us its identifier; never send \
         the full link, which contains the key.\n",
    );
    text
}

pub fn terms(f: &Facts, extra: Option<&str>) -> String {
    let mut text = String::from(
        "This service lets you share encrypted text, code and files through a link. By using it, you \
         agree to these rules.\n\n\
         ## What you may not share\n\n\
         Do not use the service for content or purposes that are unlawful, in particular content that \
         infringes the rights of others, depicts the sexual abuse of children, incites violence or \
         hatred, or serves to spread malware or to deceive people (phishing).\n\n\
         ## Reports and moderation\n\n",
    );
    if f.reports {
        text.push_str(&format!("Anyone can report a capsule on the [report page]({}). ", f.report_href));
    }
    text.push_str(
        &"As capsules are encrypted, we cannot check their content. On a justified notice, we block or \
         delete a capsule by its identifier.\n\n\
         ## Availability\n\n\
         The service is provided as it is, without a guarantee that it is available at any time. \
         Capsules are deleted according to the rules in the [privacy notice]({privacy}) and cannot \
         be restored after that; keep your own copy of anything you still need."
            .replace("{privacy}", &f.privacy_href),
    );
    if let Some(extra) = extra {
        text.push_str("\n\n");
        text.push_str(extra.trim());
    }
    text.push_str(
        "\n\n## Changes\n\n\
         We may change these rules. The version shown here applies.\n",
    );
    text
}
