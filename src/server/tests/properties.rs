#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Property-based tests for the server-side parsers that read request data or
//! configuration: paste ids, verifiers, size and duration units and report
//! validation.
//!
//! The properties are what a fuzzer would check: no input panics, accepted
//! values round-trip, and there is exactly one accepted spelling of each value.

use capsulebin_server::config::units::{ByteSize, HumanDuration};
use capsulebin_server::domain::abuse::NewReport;
use capsulebin_server::domain::{PasteId, Verifier};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1_000))]

    #[test]
    fn paste_id_parsing_never_panics(text in ".{0,64}") {
        let _ = PasteId::parse(&text);
    }

    #[test]
    fn paste_ids_round_trip_and_have_one_spelling(bytes in any::<[u8; 16]>()) {
        let id = PasteId::from_bytes(bytes);
        let text = id.encode();
        prop_assert_eq!(text.len(), 22);
        prop_assert_eq!(PasteId::parse(&text).unwrap().encode(), text);
    }

    #[test]
    fn an_accepted_id_is_always_its_own_canonical_form(text in "[A-Za-z0-9_-]{22}") {
        // Base64url with 22 characters carries 4 spare bits; a parser that ignored
        // them would accept 16 spellings of every id.
        if let Ok(id) = PasteId::parse(&text) {
            prop_assert_eq!(id.encode(), text);
        }
    }

    #[test]
    fn verifier_parsing_never_panics_and_round_trips(text in ".{0,64}", bytes in any::<[u8; 32]>()) {
        let _ = Verifier::parse(&text);
        let verifier = Verifier::from_bytes(bytes);
        prop_assert!(Verifier::parse(&verifier.encode()).unwrap().matches(&verifier));
    }

    #[test]
    fn unit_parsing_never_panics(text in ".{0,32}") {
        let _ = ByteSize::parse(&text);
        let _ = HumanDuration::parse(&text);
    }

    #[test]
    fn byte_sizes_parse_back(value in 0u64..1_000_000, unit in prop::sample::select(vec!["", "B", "KiB", "MiB", "GiB"])) {
        let factor: u64 = match unit { "KiB" => 1024, "MiB" => 1024 * 1024, "GiB" => 1024 * 1024 * 1024, _ => 1 };
        let parsed = ByteSize::parse(&format!("{value}{unit}")).unwrap();
        prop_assert_eq!(parsed.as_u64(), value * factor);
    }

    #[test]
    fn durations_parse_back(value in 1u64..100_000, unit in prop::sample::select(vec!["s", "m", "h", "d"])) {
        let factor: u64 = match unit { "m" => 60, "h" => 3600, "d" => 86_400, _ => 1 };
        let parsed = HumanDuration::parse(&format!("{value}{unit}")).unwrap();
        prop_assert_eq!(parsed.as_secs(), value * factor);
    }

    #[test]
    fn accepted_reports_carry_no_key_and_no_control_characters(
        reason in prop::sample::select(vec!["spam", "malware", "other", "nonsense"]),
        description in "\\PC{0,200}|.{0,200}",
        contact in proptest::option::of(".{0,40}"),
    ) {
        let outcome = NewReport::validate("r".into(), PasteId::from_bytes([1; 16]), reason, &description, contact.as_deref(), 0);
        if let Ok(report) = outcome {
            prop_assert!(!report.description.is_empty());
            prop_assert!(!report.description.contains("#v1."));
            prop_assert!(report.description.chars().all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')));
            if let Some(contact) = &report.contact {
                prop_assert!(contact.chars().all(|c| !c.is_control()));
                prop_assert!(!contact.contains("#v1."));
            }
        }
    }
}
