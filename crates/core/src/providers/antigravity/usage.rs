//! Parses `agy -p "/usage" --output-format json`.
//!
//! Quota is per group of models that share one limit, not per model; each
//! group lists its buckets (windows) with a remaining fraction and reset time.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{Error, QuotaSnapshot, QuotaWindow, QuotaWindowKind, Result};

pub fn quota(stdout: &[u8]) -> Result<QuotaSnapshot> {
    let payload: Value = serde_json::from_slice(stdout)
        .map_err(|_| Error::Provider("Antigravity returned unreadable /usage output.".into()))?;
    if payload["status"].as_str() != Some("SUCCESS") {
        return Err(failure(payload["error"].as_str().unwrap_or_default()));
    }
    let windows: Vec<_> = payload["command"]["data"]["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| {
            let name = group["name"].as_str().unwrap_or_default();
            let models = group["description"]
                .as_str()
                .map(|text| text.trim_start_matches("Models within this group: "));
            group["buckets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |bucket| window(name, models, bucket))
        })
        .collect();
    if windows.is_empty() {
        return Err(Error::Provider(
            "Antigravity did not return quota for this account.".into(),
        ));
    }
    Ok(QuotaSnapshot {
        windows,
        ..QuotaSnapshot::default()
    })
}

fn window(group: &str, models: Option<&str>, bucket: &Value) -> Option<QuotaWindow> {
    let remaining = bucket["remaining_fraction"].as_f64()?;
    let (kind, minutes) = match bucket["window"].as_str() {
        Some("weekly") => (QuotaWindowKind::Weekly, Some(7 * 24 * 60)),
        Some("daily") => (QuotaWindowKind::Session, Some(24 * 60)),
        // Short rolling windows such as "5h".
        Some(other) => (
            QuotaWindowKind::Session,
            other
                .strip_suffix('h')
                .and_then(|hours| hours.parse::<i64>().ok())
                .map(|hours| hours * 60),
        ),
        None => (QuotaWindowKind::Session, None),
    };
    Some(QuotaWindow {
        kind,
        label: group.to_owned(),
        used_percent: ((1.0 - remaining.clamp(0.0, 1.0)) * 100.0).round() as i64,
        resets_at: bucket["reset_time"]
            .as_str()
            .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
            .map(|time| time.with_timezone(&Utc)),
        window_duration_minutes: minutes,
        detail: models.map(str::to_owned),
    })
}

fn failure(message: &str) -> Error {
    if message.contains("not eligible") || message.contains("Verify your account") {
        Error::Provider(
            "Google asks you to verify this account before Antigravity can use it. Run `agy -p /usage` while it is active and open the link it prints.".into(),
        )
    } else if message.contains("not signed in") || message.contains("sign in") {
        Error::Provider("This Antigravity account needs to be connected again.".into())
    } else {
        Error::Provider(format!("Antigravity could not read usage: {message}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = r#"{"conversation_id":"","status":"SUCCESS","response":"x","command":{"name":"usage","data":{"groups":[
      {"name":"Gemini Models","description":"Models within this group: Gemini 3.8 Flash","buckets":[{"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":0.9604799747467041,"reset_time":"2026-12-01T13:04:55Z"},{"id":"gemini-5h","name":"Five Hour Limit Remaining","window":"5h","remaining_fraction":0.5,"reset_time":"2026-11-25T20:57:48Z"}]},
      {"name":"Claude and GPT models","description":"Models within this group: Claude Sonnet 4.6, Claude Opus 4.6, GPT-OSS 120B","buckets":[{"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-12-01T13:07:31Z"},{"id":"3p-5h","name":"Five Hour Limit Remaining","window":"5h","remaining_fraction":0.25,"reset_time":"2026-11-25T20:57:48Z"}]}]}}}"#;

    #[test]
    fn each_group_gets_a_weekly_and_a_five_hour_window_with_its_models() {
        let quota = quota(REAL.as_bytes()).unwrap();
        assert_eq!(quota.windows.len(), 4);
        let gemini = &quota.windows[0];
        assert_eq!(gemini.label, "Gemini Models");
        assert_eq!(gemini.used_percent, 4);
        assert_eq!(gemini.kind, QuotaWindowKind::Weekly);
        assert_eq!(gemini.window_duration_minutes, Some(10080));
        assert_eq!(gemini.detail.as_deref(), Some("Gemini 3.8 Flash"));
        assert!(gemini.resets_at.is_some());
        let five_hour = &quota.windows[1];
        assert_eq!(five_hour.label, "Gemini Models");
        assert_eq!(five_hour.kind, QuotaWindowKind::Session);
        assert_eq!(five_hour.window_duration_minutes, Some(300));
        assert_eq!(five_hour.used_percent, 50);
        let claude = &quota.windows[2];
        assert_eq!(claude.used_percent, 0);
        assert_eq!(
            claude.detail.as_deref(),
            Some("Claude Sonnet 4.6, Claude Opus 4.6, GPT-OSS 120B")
        );
        // The summaries follow the group closest to its limit.
        assert_eq!(quota.weekly().unwrap().label, "Gemini Models");
        assert_eq!(quota.session().unwrap().label, "Claude and GPT models");
        assert_eq!(quota.session().unwrap().used_percent, 75);
    }

    #[test]
    fn errors_name_the_account_problem() {
        let ineligible = br#"{"status":"ERROR","error":"Eligibility check failed: Your current account is not eligible for Antigravity. Verify your account to continue."}"#;
        assert!(
            quota(ineligible)
                .unwrap_err()
                .to_string()
                .contains("verify this account")
        );
        assert!(quota(b"not json").is_err());
        assert!(quota(br#"{"status":"SUCCESS","command":{"data":{"groups":[]}}}"#).is_err());
    }
}
