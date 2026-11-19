//! Configuration tests.
//!
//! The focus is on rejection. A configuration that parses is not the interesting
//! case; a configuration that parses *and is wrong* is, because that is what
//! reaches production.

use super::*;

/// A minimal configuration that validates. Tests derive from this so that each
/// one changes exactly the thing it is about.
fn valid(directory: &Path) -> Config {
    Config {
        server: ServerConfig {
            bind: "127.0.0.1:8080".to_owned(),
            public_url: Some("https://paste.example".to_owned()),
            asset_dir: None,
            trusted_proxies: Vec::new(),
        },
        database: DatabaseConfig {
            path: directory.join("capsulebin.db"),
            ..DatabaseConfig::default()
        },
        pastes: PastesConfig::default(),
        creation: CreationConfig {
            mode: CreationMode::Token,
            token_hashes: vec![format!("sha256:{}", Verifier::for_creation_token(b"token").encode())],
        },
        purge: PurgeConfig::default(),
        rate_limits: Default::default(),
        abuse: Default::default(),
        legal: Default::default(),
    }
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("capsulebin-config-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn error(config: &Config) -> String {
    match config.validate() {
        Ok(_) => panic!("configuration should have been rejected"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn accepts_a_valid_configuration() {
    let dir = tempdir("valid");
    let resolved = valid(&dir).validate().expect("valid configuration");

    assert_eq!(resolved.creation_tokens.len(), 1);
    assert!(resolved.max_body_bytes > resolved.config.pastes.max_plaintext_size.as_usize());
}

/// The default has to be safe on its own, because it is what a half-configured
/// instance runs with.
#[test]
fn default_creation_mode_is_token() {
    assert_eq!(CreationConfig::default().mode, CreationMode::Token);
}

#[test]
fn requires_public_url() {
    let dir = tempdir("no-url");
    let mut config = valid(&dir);
    config.server.public_url = None;

    assert!(error(&config).contains("public_url"));
}

#[test]
fn rejects_relative_or_trailing_slash_public_url() {
    let dir = tempdir("bad-url");

    for bad in ["paste.example", "/paste", "https://paste.example/"] {
        let mut config = valid(&dir);
        config.server.public_url = Some(bad.to_owned());
        assert!(config.validate().is_err(), "public_url {bad:?} should be rejected");
    }
}

#[test]
fn rejects_unparsable_bind_address() {
    let dir = tempdir("bad-bind");
    let mut config = valid(&dir);
    config.server.bind = "not-an-address".to_owned();

    assert!(error(&config).contains("bind"));
}

/// A default longer than the maximum means every unspecified paste would be
/// clamped, which is a configuration mistake rather than an intent.
#[test]
fn rejects_default_expiry_above_maximum() {
    let dir = tempdir("expiry");
    let mut config = valid(&dir);
    config.pastes.default_expiry = HumanDuration::parse("90d").expect("90d");
    config.pastes.maximum_expiry = HumanDuration::parse("7d").expect("7d");

    assert!(error(&config).contains("exceeds"));
}

#[test]
fn rejects_zero_expiry() {
    let dir = tempdir("zero-expiry");

    for (default, maximum) in [("0s", "7d"), ("7d", "0s")] {
        let mut config = valid(&dir);
        config.pastes.default_expiry = HumanDuration::parse(default).expect("duration");
        config.pastes.maximum_expiry = HumanDuration::parse(maximum).expect("duration");
        assert!(config.validate().is_err(), "{default}/{maximum} should be rejected");
    }
}

#[test]
fn rejects_absurd_or_zero_size_limits() {
    let dir = tempdir("sizes");

    for bad in ["0", "1GiB"] {
        let mut config = valid(&dir);
        config.pastes.max_plaintext_size = ByteSize::parse(bad).expect("size");
        let outcome = config.validate();
        if bad == "0" {
            assert!(outcome.is_err(), "zero size must be rejected");
        } else {
            // 1 GiB is above the supported ceiling.
            assert!(outcome.is_err(), "{bad} should exceed the supported maximum");
        }
    }
}

/// The public header is handed back on an unauthenticated descriptor request, so a
/// generous limit is an invitation to store something else in it.
#[test]
fn rejects_an_oversized_public_header_limit() {
    let dir = tempdir("header-limit");

    for bad in ["0", "8KiB", "1MiB"] {
        let mut config = valid(&dir);
        config.pastes.max_public_header_size = ByteSize::parse(bad).expect("size");
        assert!(error(&config).contains("max_public_header_size"), "{bad} should be rejected");
    }

    // The documented ceiling itself is accepted.
    let mut config = valid(&dir);
    config.pastes.max_public_header_size = ByteSize::parse("4KiB").expect("4KiB");
    assert!(config.validate().is_ok());
}

#[test]
fn rejects_zero_purge_durations() {
    let dir = tempdir("purge-zero");

    for name in ["never_retrieved_after", "inactive_after", "retrieval_timestamp_granularity"] {
        let mut config = valid(&dir);
        let zero = HumanDuration::parse("0s").expect("0s");
        match name {
            "never_retrieved_after" => config.purge.never_retrieved_after = zero,
            "inactive_after" => config.purge.inactive_after = zero,
            _ => config.purge.retrieval_timestamp_granularity = zero,
        }

        assert!(error(&config).contains(name), "purge.{name} = 0 should be rejected");
    }
}

/// A granularity coarser than the inactivity window would leave the timestamp
/// behind the very threshold it is compared against, and the purge would delete
/// capsules that are still being fetched.
#[test]
fn rejects_a_granularity_above_the_inactivity_window() {
    let dir = tempdir("purge-granularity");
    let mut config = valid(&dir);
    config.purge.inactive_after = HumanDuration::parse("1h").expect("1h");
    config.purge.retrieval_timestamp_granularity = HumanDuration::parse("2h").expect("2h");

    assert!(error(&config).contains("retrieval_timestamp_granularity"));
}

#[test]
fn rejects_an_unusable_purge_batch_size() {
    let dir = tempdir("purge-batch");

    for bad in [0, -1, 10_001] {
        let mut config = valid(&dir);
        config.purge.batch_size = bad;
        assert!(error(&config).contains("batch_size"), "{bad} should be rejected");
    }
}

/// The purge interval is not configurable, and this test is what keeps it that way:
/// the rejection comes from `deny_unknown_fields`, so nothing in `validate` would
/// notice if someone added the field.
#[test]
fn rejects_a_purge_interval() {
    let error = toml::from_str::<Config>(
        r#"
            [purge]
            enabled = true
            interval = "1h"
        "#,
    )
    .expect_err("purge.interval must not be configurable");

    assert!(
        error.to_string().contains("interval") || error.to_string().contains("unknown field"),
        "unexpected error: {error}"
    );
}

/// Token mode with no tokens would reject every creation attempt. That looks
/// like an outage, so it has to fail at startup instead.
#[test]
fn rejects_token_mode_without_tokens() {
    let dir = tempdir("no-tokens");
    let mut config = valid(&dir);
    config.creation.token_hashes.clear();

    assert!(error(&config).contains("token_hashes is empty"));
}

/// The reverse is also a mistake worth catching: tokens configured in public mode
/// suggest the operator believes access is restricted when it is not.
#[test]
fn rejects_tokens_in_public_mode() {
    let dir = tempdir("public-tokens");
    let mut config = valid(&dir);
    config.creation.mode = CreationMode::Public;

    assert!(error(&config).contains("would have no effect"));
}

#[test]
fn accepts_public_and_disabled_without_tokens() {
    let dir = tempdir("modes");

    for mode in [CreationMode::Public, CreationMode::Disabled] {
        let mut config = valid(&dir);
        config.creation.mode = mode;
        config.creation.token_hashes.clear();
        assert!(config.validate().is_ok(), "{mode:?} should be accepted");
    }
}

#[test]
fn rejects_malformed_token_hashes() {
    let dir = tempdir("bad-hashes");

    for bad in ["deadbeef", "sha256:not-base64!!", "md5:abc", ""] {
        let mut config = valid(&dir);
        config.creation.token_hashes = vec![bad.to_owned()];
        assert!(config.validate().is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn rejects_missing_database_directory() {
    let dir = tempdir("missing-db");
    let mut config = valid(&dir);
    config.database.path = dir.join("does/not/exist/capsulebin.db");

    assert!(error(&config).contains("does not exist"));
}

#[test]
fn rejects_zero_busy_timeout() {
    let dir = tempdir("busy");
    let mut config = valid(&dir);
    config.database.busy_timeout_ms = 0;

    assert!(error(&config).contains("busy_timeout_ms"));
}

/// A typo in a key name must not be ignored. This is the single most valuable
/// property of the parser: the operator finds out at startup, not from an
/// incident.
#[test]
fn rejects_unknown_fields() {
    let toml = r#"
        [pastes]
        default_expiry = "7d"
        maximum_expiry = "90d"
        allow_permanent = false
        max_plaintext_size = "10MiB"
        burn_after_retreival = true
    "#;

    let error = toml::from_str::<Config>(toml).expect_err("typo should be rejected");
    assert!(
        error.to_string().contains("burn_after_retreival") || error.to_string().contains("unknown field"),
        "unexpected error: {error}"
    );
}

/// A table that names one field must not require the rest.
///
/// This is what the environment overlay produces: setting
/// `CAPSULEBIN__DATABASE__PATH` creates a `[database]` table containing only
/// `path`. Without per-field defaults serde would demand every sibling, and the
/// documented way to configure a container would fail at startup.
#[test]
fn partial_tables_fall_back_to_defaults_per_field() {
    let config: Config = toml::from_str(
        r#"
            [server]
            public_url = "https://paste.example"

            [database]
            path = "/tmp/only-the-path.db"

            [pastes]
            allow_permanent = true

            [purge]
            inactive_after = "14d"
        "#,
    )
    .expect("a partial configuration must load");

    // Named values win.
    assert_eq!(config.database.path, PathBuf::from("/tmp/only-the-path.db"));
    assert!(config.pastes.allow_permanent);
    assert_eq!(config.purge.inactive_after.as_secs(), 14 * 86_400);

    // Unnamed siblings keep their defaults rather than becoming zero values.
    assert_eq!(config.database.busy_timeout_ms, 5_000);
    assert_eq!(config.database.synchronous, Synchronous::Full);
    assert_eq!(config.pastes.default_expiry.as_secs(), 604_800);
    assert_eq!(config.pastes.max_plaintext_size.as_u64(), 10 * 1024 * 1024);
    assert_eq!(config.pastes.max_public_header_size.as_u64(), 2 * 1024);
    assert_eq!(config.server.bind, "127.0.0.1:8080");
    assert!(config.purge.enabled);
    assert_eq!(config.purge.never_retrieved_after.as_secs(), 30 * 86_400);
    assert_eq!(config.purge.retrieval_timestamp_granularity.as_secs(), 3_600);
    assert_eq!(config.purge.batch_size, 500);
    // And the safe default survives a partial file.
    assert_eq!(config.creation.mode, CreationMode::Token);
}

/// The same path the acceptance script takes: configuration entirely from the
/// environment, with no file at all.
#[test]
fn a_full_configuration_can_come_from_the_environment_alone() {
    let dir = tempdir("env-only");
    let mut table = toml::Table::new();

    apply_env(
        &mut table,
        [
            ("CAPSULEBIN__SERVER__PUBLIC_URL".to_owned(), "https://paste.example".to_owned()),
            (
                "CAPSULEBIN__DATABASE__PATH".to_owned(),
                dir.join("capsulebin.db").display().to_string(),
            ),
            ("CAPSULEBIN__CREATION__MODE".to_owned(), "public".to_owned()),
        ],
    )
    .expect("overlay applies");

    let config: Config = table.try_into().expect("deserialise");
    let resolved = config.validate().expect("must be usable");

    assert_eq!(resolved.config.creation.mode, CreationMode::Public);
    assert_eq!(resolved.config.database.busy_timeout_ms, 5_000);
}

#[test]
fn env_overlay_sets_nested_values() {
    let mut table = toml::Table::new();
    apply_env(
        &mut table,
        [
            ("CAPSULEBIN__SERVER__BIND".to_owned(), "0.0.0.0:9000".to_owned()),
            ("CAPSULEBIN__PURGE__BATCH_SIZE".to_owned(), "8".to_owned()),
            ("CAPSULEBIN__PASTES__ALLOW_PERMANENT".to_owned(), "true".to_owned()),
            ("UNRELATED".to_owned(), "ignored".to_owned()),
        ],
    )
    .expect("overlay applies");

    let server = table.get("server").and_then(toml::Value::as_table).expect("server table");
    assert_eq!(server.get("bind").and_then(toml::Value::as_str), Some("0.0.0.0:9000"));

    let purge = table.get("purge").and_then(toml::Value::as_table).expect("purge table");
    // Types must survive: a string "8" would fail to deserialise into an integer.
    assert_eq!(purge.get("batch_size").and_then(toml::Value::as_integer), Some(8));
    let pastes = table.get("pastes").and_then(toml::Value::as_table).expect("pastes table");
    assert_eq!(pastes.get("allow_permanent").and_then(toml::Value::as_bool), Some(true));

    assert!(!table.contains_key("unrelated"));
}

/// A bind address contains a colon and is not valid TOML on its own, so the
/// fallback to a string is what makes the common case work.
#[test]
fn env_values_that_are_not_toml_become_strings() {
    assert_eq!(parse_scalar("0.0.0.0:8080"), toml::Value::String("0.0.0.0:8080".to_owned()));
    assert_eq!(parse_scalar("7d"), toml::Value::String("7d".to_owned()));
}

#[test]
fn env_overlay_parses_arrays() {
    let value = parse_scalar(r#"["sha256:aaa", "sha256:bbb"]"#);
    let array = value.as_array().expect("array");
    assert_eq!(array.len(), 2);
}

#[test]
fn env_overlay_rejects_empty_segments() {
    let mut table = toml::Table::new();
    let outcome = apply_env(&mut table, [("CAPSULEBIN__SERVER____BIND".to_owned(), "x".to_owned())]);

    assert!(outcome.is_err());
}

/// The body limit has to exceed the plaintext limit, because base64 inflates the
/// payload by a third. Getting this wrong would reject every maximum-size paste.
#[test]
fn body_limit_covers_base64_overhead() {
    let plaintext = ByteSize::parse("10MiB").expect("10MiB");
    let limit = max_body_bytes(plaintext);

    let expected_minimum = plaintext.as_usize() * 4 / 3;
    assert!(
        limit >= expected_minimum,
        "body limit {limit} is below the base64-inflated payload {expected_minimum}"
    );
}

#[test]
fn missing_config_file_is_an_error_but_no_path_is_fine() {
    let dir = tempdir("load");

    let outcome = Config::load(Some(&dir.join("absent.toml")));
    assert!(outcome.is_err(), "an explicitly named missing file must fail");

    assert!(Config::load(None).is_ok(), "defaults alone must load");
}

#[test]
fn loads_from_a_file() {
    let dir = tempdir("from-file");
    let path = dir.join("capsulebin.toml");
    std::fs::write(
        &path,
        r#"
            [server]
            bind = "127.0.0.1:1234"
            public_url = "https://paste.example"

            [creation]
            mode = "public"
        "#,
    )
    .expect("write config");

    let config = Config::load(Some(&path)).expect("load");
    assert_eq!(config.server.bind, "127.0.0.1:1234");
    assert_eq!(config.creation.mode, CreationMode::Public);
    // Untouched tables fall back to their defaults.
    assert_eq!(config.pastes.default_expiry.as_secs(), 604_800);
}

#[test]
fn abuse_and_report_rate_limits_load_and_validate() {
    let dir = tempdir("abuse");
    let path = dir.join("capsulebin.toml");
    std::fs::write(
        &path,
        r#"
            [abuse]
            enabled = true
            contact = "abuse@paste.example"
            report_retention = "30d"

            [rate_limits.report]
            requests_per_hour = 5
            burst = 2
        "#,
    )
    .expect("write config");

    let config = Config::load(Some(&path)).expect("load");
    assert_eq!(config.abuse.contact.as_deref(), Some("abuse@paste.example"));
    assert_eq!(config.abuse.report_retention.as_secs(), 30 * 86_400);
    assert_eq!(config.rate_limits.report.requests_per_hour, 5);
}

#[test]
fn rejects_invalid_abuse_settings() {
    let dir = tempdir("abuse-invalid");

    let mut config = valid(&dir);
    config.abuse.report_retention = crate::config::units::HumanDuration(std::time::Duration::ZERO);
    assert!(config.validate().is_err(), "zero retention accepted");

    let mut config = valid(&dir);
    config.abuse.contact = Some("  ".to_owned());
    assert!(config.validate().is_err(), "blank contact accepted");

    let mut config = valid(&dir);
    config.abuse.contact = Some("a\nb".to_owned());
    assert!(config.validate().is_err(), "contact with a line break accepted");
}

#[test]
fn legal_sections_can_be_set_from_the_environment() {
    let mut table = toml::Table::new();
    apply_env(
        &mut table,
        [
            (
                "CAPSULEBIN__LEGAL__OPERATOR__FILE".to_owned(),
                "/etc/capsulebin/operator.md".to_owned(),
            ),
            (
                "CAPSULEBIN__LEGAL__PRIVACY__EXTRA".to_owned(),
                "Backups are kept for 7 days.".to_owned(),
            ),
        ],
    )
    .expect("overlay");
    let config: Config = table.try_into().expect("parse");

    assert_eq!(
        config.legal.operator.and_then(|operator| operator.file),
        Some(PathBuf::from("/etc/capsulebin/operator.md"))
    );
    assert_eq!(
        config.legal.privacy.and_then(|privacy| privacy.extra).as_deref(),
        Some("Backups are kept for 7 days.")
    );
}

#[test]
fn rejects_unknown_legal_keys() {
    let parsed = toml::from_str::<Config>("[legal.operator]\nimprint = \"x\"\n");
    assert!(parsed.is_err(), "a typo in a legal section must not be ignored");
}

#[test]
fn rejects_ambiguous_or_unusable_rate_limits() {
    let dir = tempdir("rate-invalid");

    let mut config = valid(&dir);
    config.rate_limits.report.requests_per_minute = 1;
    config.rate_limits.report.requests_per_hour = 5;
    config.rate_limits.report.burst = 1;
    assert!(config.validate().is_err(), "both rates accepted");

    let mut config = valid(&dir);
    config.rate_limits.create.requests_per_minute = 5;
    config.rate_limits.create.burst = 0;
    assert!(config.validate().is_err(), "a rate without burst accepted");
}

/// `max_items` and `allow_binary_files` used to be accepted and silently did
/// nothing: the server cannot see inside a capsule. A file that still sets them
/// must fail loudly rather than suggest a limit that is not enforced.
#[test]
fn removed_content_limits_are_rejected() {
    for key in ["max_items = 8", "allow_binary_files = false"] {
        let toml = format!("[pastes]\n{key}\n");
        assert!(toml::from_str::<Config>(&toml).is_err(), "{key} was accepted");
    }
}
