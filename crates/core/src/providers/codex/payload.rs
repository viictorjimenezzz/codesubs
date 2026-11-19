use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{
    CreditBalance, QuotaSnapshot, QuotaWindow, QuotaWindowKind, ResetCredits, ResetOutcome,
};

pub fn account(payload: &Value) -> Option<(Option<String>, Option<String>)> {
    let account = payload.get("account")?;
    if account.get("type")?.as_str()? != "chatgpt" {
        return None;
    }
    Some((string(account, "email"), string(account, "planType")))
}

pub fn quota(payload: &Value) -> Option<QuotaSnapshot> {
    let snapshots = payload
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object);
    let snapshot = snapshots
        .and_then(|map| map.get("codex").or_else(|| map.values().next()))
        .or_else(|| payload.get("rateLimits"))?;
    let windows: Vec<_> = ["primary", "secondary"]
        .iter()
        .filter_map(|key| snapshot.get(key).and_then(window))
        .collect();
    if windows.is_empty() {
        return None;
    }
    Some(QuotaSnapshot {
        windows,
        reset_credits: payload.get("rateLimitResetCredits").and_then(reset_credits),
        credits: snapshot.get("credits").and_then(credits),
    })
}

fn window(value: &Value) -> Option<QuotaWindow> {
    let used_percent = integer(value.get("usedPercent")?)?;
    let window_duration_minutes = value.get("windowDurationMins").and_then(integer);
    let (kind, label) = match window_duration_minutes {
        Some(minutes) if minutes >= 6 * 24 * 60 => (QuotaWindowKind::Weekly, "Week".into()),
        Some(minutes) if minutes > 0 && minutes % 60 == 0 => {
            (QuotaWindowKind::Session, format!("{}h", minutes / 60))
        }
        Some(minutes) if minutes > 0 => (QuotaWindowKind::Session, format!("{minutes}m")),
        _ => (QuotaWindowKind::Session, "Session".into()),
    };
    Some(QuotaWindow {
        kind,
        label,
        used_percent,
        resets_at: value.get("resetsAt").and_then(timestamp),
        window_duration_minutes,
    })
}

fn credits(value: &Value) -> Option<CreditBalance> {
    let value = value.as_object()?;
    let unlimited = value
        .get("unlimited")
        .and_then(Value::as_bool)
        .unwrap_or_default();
    let balance = value
        .get("balance")
        .and_then(|balance| balance.as_f64().or_else(|| balance.as_str()?.parse().ok()))
        .filter(|balance| balance.is_finite());
    if unlimited
        || value.get("hasCredits").and_then(Value::as_bool) == Some(true)
        || balance.is_some()
    {
        Some(CreditBalance { balance, unlimited })
    } else {
        None
    }
}

fn reset_credits(value: &Value) -> Option<ResetCredits> {
    let value = value.as_object()?;
    let available_credits = value
        .get("credits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|credit| credit["status"].as_str() == Some("available"));
    let available = value
        .get("availableCount")
        .and_then(integer)
        .unwrap_or_else(|| available_credits.clone().count() as i64);
    let next_expiry = available_credits
        .filter_map(|credit| credit.get("expiresAt").and_then(timestamp))
        .min();
    Some(ResetCredits {
        available,
        next_expiry,
        grants: None,
    })
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(integer(value)?, 0)
}

fn integer(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    let number = value.as_f64()?;
    if number >= i64::MIN as f64 && number < -(i64::MIN as f64) {
        Some(number as i64)
    } else {
        None
    }
}

pub fn reset_outcome(result: &Value) -> Option<ResetOutcome> {
    Some(match result.get("outcome")?.as_str()? {
        "reset" => ResetOutcome::Reset,
        "nothingToReset" => ResetOutcome::NothingToReset,
        "noCredit" => ResetOutcome::NoCredit,
        "alreadyRedeemed" => ResetOutcome::AlreadyRedeemed,
        other => ResetOutcome::Other(other.to_owned()),
    })
}
