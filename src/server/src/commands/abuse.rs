//! `capsulebin reports …` and `capsulebin moderation …`: the operator side of
//! notice and action (ADR-013).
//!
//! There is no admin web interface. These commands work on the database
//! directly, so they need the same configuration as `serve` and can run while
//! the server is up (SQLite WAL, one writer at a time).
//!
//! Report text is shown in full here — the operator needs it — but never logged.
//! Control characters were rejected on submission, so printing it to a terminal
//! cannot inject escape sequences.

use anyhow::{Context as _, bail};
use capsulebin_server::config::Resolved;
use capsulebin_server::domain::abuse::{ModerationAction, ReasonCode, ReportStatus, Resolution};
use capsulebin_server::domain::{Availability, Clock as _, PasteId, SystemClock};
use capsulebin_server::repository::{AbuseRepository as _, ModerationOutcome, PasteRepository as _, ReportCeilings, ReportEntry};

/// Formats Unix seconds as `YYYY-MM-DD HH:MM UTC`.
///
/// Civil-from-days after Howard Hinnant; exact for the proleptic Gregorian
/// calendar, which is all a log-style timestamp needs. Avoids a date crate for
/// one display function.
pub(crate) fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, minute) = (rem / 3600, (rem % 3600) / 60);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

fn codes<T: Copy>(all: &[T], as_str: fn(T) -> &'static str) -> String {
    all.iter().map(|c| as_str(*c)).collect::<Vec<_>>().join(", ")
}

fn parse_reason(text: &str) -> anyhow::Result<ReasonCode> {
    ReasonCode::parse(text).with_context(|| {
        format!(
            "unknown reason {text:?}; expected one of: {}",
            codes(ReasonCode::ALL, ReasonCode::as_str)
        )
    })
}

fn parse_paste_id(text: &str) -> anyhow::Result<PasteId> {
    // Accept a pasted link too, but only ever use the id: strip the fragment
    // (the key) before anything else looks at the text.
    let without_fragment = text.split('#').next().unwrap_or_default();
    let candidate = without_fragment.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    PasteId::parse(candidate).map_err(|_| anyhow::anyhow!("not a paste id: expected 22 base64url characters"))
}

fn entry_json(entry: &ReportEntry) -> serde_json::Value {
    let r = &entry.report;
    serde_json::json!({
        "id": r.id,
        "paste_id": r.paste_id,
        "paste_status": entry.paste_status.as_deref().unwrap_or("gone"),
        "reason": r.reason,
        "description": r.description,
        "contact": r.contact,
        "created_at": r.created_at,
        "status": r.status,
        "handled_at": r.handled_at,
        "resolution": r.resolution,
    })
}

/// The line `reports list` starts with: the open reports against both
/// ceilings, and whether the instance currently drops new reports.
fn open_reports_summary(open: i64, ceilings: ReportCeilings) -> String {
    let mut summary = format!(
        "open reports: {open} of {} (at most {} per capsule)",
        ceilings.total, ceilings.per_paste
    );
    if open >= ceilings.total {
        summary.push_str("\nthe limit is reached: new reports are not stored until open ones are closed");
    }
    summary
}

pub(crate) async fn reports_list(resolved: Resolved, status: String, limit: i64, json: bool) -> anyhow::Result<()> {
    let filter = match status.as_str() {
        "all" => None,
        other => Some(ReportStatus::parse(other).with_context(|| {
            format!(
                "unknown status {other:?}; expected all, {}",
                codes(ReportStatus::ALL, ReportStatus::as_str)
            )
        })?),
    };
    anyhow::ensure!((1..=10_000).contains(&limit), "--limit must be between 1 and 10000");

    let state = super::state_for(&resolved).await?;
    let entries = state
        .abuse_repo
        .list_reports(filter, limit)
        .await
        .context("could not read reports")?;

    if json {
        let list: Vec<_> = entries.iter().map(entry_json).collect();
        println!("{}", serde_json::Value::Array(list));
        return Ok(());
    }

    let open = state
        .abuse_repo
        .count_open_reports()
        .await
        .context("could not count open reports")?;
    println!("{}", open_reports_summary(open, state.abuse.report_ceilings()));

    if entries.is_empty() {
        println!("no {} reports", status);
        return Ok(());
    }
    println!(
        "{:<26}  {:<20}  {:<9}  {:<15}  {:<22}  PASTE",
        "REPORT", "RECEIVED", "STATUS", "REASON", "PASTE ID"
    );
    for entry in &entries {
        let r = &entry.report;
        println!(
            "{:<26}  {:<20}  {:<9}  {:<15}  {:<22}  {}",
            r.id,
            format_utc(r.created_at),
            r.status,
            r.reason,
            r.paste_id,
            entry.paste_status.as_deref().unwrap_or("gone"),
        );
    }
    Ok(())
}

pub(crate) async fn reports_show(resolved: Resolved, id: String, json: bool) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;
    let Some(entry) = state.abuse_repo.find_report(&id).await.context("could not read the report")? else {
        bail!("no report {id}");
    };

    if json {
        println!("{}", entry_json(&entry));
        return Ok(());
    }

    let r = &entry.report;
    println!("Report      {}", r.id);
    println!("Received    {}", format_utc(r.created_at));
    println!("Status      {}", r.status);
    if let (Some(at), Some(resolution)) = (r.handled_at, &r.resolution) {
        println!("Closed      {} ({resolution})", format_utc(at));
    }
    println!("Reason      {}", r.reason);
    println!("Paste       {} ({})", r.paste_id, entry.paste_status.as_deref().unwrap_or("gone"));
    println!("Contact     {}", r.contact.as_deref().unwrap_or("-"));
    println!();
    for line in r.description.lines() {
        println!("  {line}");
    }
    Ok(())
}

pub(crate) async fn reports_review(resolved: Resolved, id: String) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;
    if !state.abuse_repo.review_report(&id).await.context("could not update the report")? {
        bail!("report {id} does not exist or is not open");
    }
    println!("ok: report {id} is being reviewed");
    Ok(())
}

pub(crate) async fn reports_close(resolved: Resolved, id: String, resolution: String) -> anyhow::Result<()> {
    let resolution = Resolution::parse(&resolution).with_context(|| {
        format!(
            "unknown resolution {resolution:?}; expected one of: {}",
            codes(Resolution::ALL, Resolution::as_str)
        )
    })?;
    let state = super::state_for(&resolved).await?;
    let now = SystemClock.now();
    if !state
        .abuse_repo
        .close_report(&id, resolution, now)
        .await
        .context("could not update the report")?
    {
        bail!("report {id} does not exist or is already closed");
    }
    println!("ok: report {id} closed ({resolution})");
    Ok(())
}

pub(crate) async fn moderate(
    resolved: Resolved,
    action: ModerationAction,
    paste: String,
    reason: Option<String>,
    note: Option<String>,
) -> anyhow::Result<()> {
    let paste_id = parse_paste_id(&paste)?;
    let reason = reason.as_deref().map(parse_reason).transpose()?;
    if let Some(note) = &note {
        anyhow::ensure!(
            note.len() <= capsulebin_server::domain::abuse::MAX_NOTE_BYTES,
            "--note is longer than {} bytes",
            capsulebin_server::domain::abuse::MAX_NOTE_BYTES
        );
    }

    let state = super::state_for(&resolved).await?;
    let now = SystemClock.now();
    let outcome = state
        .abuse_repo
        .moderate(&paste_id, action, reason, note.as_deref(), now)
        .await
        .context("could not apply the moderation action")?;

    let id = paste_id.encode();
    match outcome {
        ModerationOutcome::Applied => {
            let done = match action {
                ModerationAction::Disable => "disabled (payload kept; `moderation restore` reverses it)",
                ModerationAction::Delete => "deleted (payload removed; this cannot be undone)",
                ModerationAction::Restore => "restored",
            };
            println!("ok: paste {id} {done}");
            Ok(())
        }
        ModerationOutcome::NotFound => bail!("no paste {id} (never existed, or already purged)"),
        ModerationOutcome::NotApplicable(current) => {
            let hint = match (action, current) {
                (ModerationAction::Restore, Availability::Active) => "it is not disabled",
                (_, Availability::Deleted) => "it is deleted; nothing can be done to it any more",
                (ModerationAction::Disable, Availability::Disabled) => "it is already disabled",
                _ => "it is in the wrong state",
            };
            bail!("cannot {action} paste {id}: {hint} (status: {})", current.as_str())
        }
    }
}

pub(crate) async fn moderation_status(resolved: Resolved, paste: String, json: bool) -> anyhow::Result<()> {
    let paste_id = parse_paste_id(&paste)?;
    let state = super::state_for(&resolved).await?;
    let Some(found) = state.pastes_repo.find(&paste_id).await.context("could not read the paste")? else {
        bail!("no paste {} (never existed, or already purged)", paste_id.encode());
    };
    let history = state
        .abuse_repo
        .moderation_history(&paste_id)
        .await
        .context("could not read the moderation history")?;
    let now = SystemClock.now();

    if json {
        let actions: Vec<_> = history
            .iter()
            .map(|h| serde_json::json!({"action": h.action, "reason": h.reason, "note": h.note, "at": h.created_at}))
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "id": found.id.encode(),
                "status": found.availability.as_str(),
                "retrievable": found.is_retrievable(now),
                "created_at": found.created_at,
                "expires_at": found.expires_at,
                "burn_after_retrieval": found.burn_after_retrieval,
                "consumed": found.is_consumed(),
                "payload_size": found.payload_size,
                "moderation": actions,
            })
        );
        return Ok(());
    }

    println!("Paste       {}", found.id.encode());
    println!(
        "Status      {}{}",
        found.availability.as_str(),
        if found.is_retrievable(now) { " (retrievable)" } else { "" }
    );
    println!("Created     {}", format_utc(found.created_at));
    println!("Expires     {}", found.expires_at.map_or_else(|| "never".to_owned(), format_utc));
    if found.burn_after_retrieval {
        println!("Burn        {}", if found.is_consumed() { "consumed" } else { "not yet retrieved" });
    }
    println!("Size        {} bytes of ciphertext", found.payload_size);
    if history.is_empty() {
        println!("Moderation  none");
    }
    for h in &history {
        println!(
            "Moderation  {}  {}{}{}",
            format_utc(h.created_at),
            h.action,
            h.reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default(),
            h.note.as_deref().map(|n| format!(" - {n}")).unwrap_or_default(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(format_utc(1_795_434_240), "2026-11-23 11:44 UTC");
        assert_eq!(format_utc(-60), "1969-12-31 23:59 UTC");
    }

    #[test]
    fn the_report_list_starts_with_the_open_reports_and_their_ceilings() {
        let ceilings = ReportCeilings {
            per_paste: 10,
            total: 1_000,
        };
        assert_eq!(
            open_reports_summary(0, ceilings),
            "open reports: 0 of 1000 (at most 10 per capsule)"
        );
        assert_eq!(
            open_reports_summary(3, ceilings),
            "open reports: 3 of 1000 (at most 10 per capsule)"
        );
        assert!(!open_reports_summary(999, ceilings).contains("new reports are not stored"));

        let full = open_reports_summary(1_000, ceilings);
        assert!(full.starts_with("open reports: 1000 of 1000 (at most 10 per capsule)\n"), "{full}");
        assert!(full.contains("new reports are not stored"), "{full}");
    }

    #[test]
    fn a_pasted_link_is_reduced_to_its_id() {
        // Canonical: the last character carries no padding bits.
        let id = "AbCdEfGhIjKlMnOpQrStUQ";
        for input in [
            id.to_owned(),
            format!("https://paste.example/p/{id}"),
            format!("https://paste.example/p/{id}#v1.c2VjcmV0"),
            format!("https://paste.example/manage/{id}/"),
        ] {
            assert_eq!(parse_paste_id(&input).map(|p| p.encode()).ok().as_deref(), Some(id), "{input}");
        }
        assert!(parse_paste_id("short").is_err());
    }
}
