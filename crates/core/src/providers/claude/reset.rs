use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{ResetCredits, ResetGrant};

/// Missing, malformed and ineligible responses leave the balance unknown.
pub fn parse(payload: &Value, now: DateTime<Utc>) -> Option<ResetCredits> {
    let status = payload.get("cedar_ember")?;
    if !status.get("eligible")?.as_bool()? {
        return None;
    }
    let grants = status
        .get("grants")?
        .as_array()?
        .iter()
        .map(grant)
        .collect::<Option<Vec<_>>>()?;
    let active = grants.iter().filter(|grant| grant.is_available_at(now));
    let available = active
        .clone()
        .try_fold(0_i64, |total, grant| total.checked_add(grant.available))?;
    let next_expiry = active.filter_map(|grant| grant.expires_at).min();
    Some(ResetCredits {
        available,
        next_expiry,
        grants: Some(grants),
    })
}

fn grant(value: &Value) -> Option<ResetGrant> {
    let label = value.get("label")?.as_str()?;
    let count = value.get("resets_left")?.as_f64()?;
    if label.is_empty() || !(0.0..=1_000_000.0).contains(&count) || count.fract() != 0.0 {
        return None;
    }
    Some(ResetGrant {
        label: label.to_owned(),
        available: count as i64,
        starts_at: date(value.get("starts_at"))?,
        expires_at: date(value.get("ends_at"))?,
        paused: value.get("paused")?.as_bool()?,
    })
}

fn date(value: Option<&Value>) -> Option<Option<DateTime<Utc>>> {
    match value {
        None | Some(Value::Null) => Some(None),
        Some(value) => DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()
            .map(|date| Some(date.to_utc())),
    }
}

#[cfg(target_os = "macos")]
pub(super) async fn read(home: &std::path::Path, cli_version: &str) -> Option<ResetCredits> {
    use std::time::Duration;

    use crate::platform::keychain;
    use crate::providers::CredentialLocation;

    const BODY_LIMIT: usize = 1024 * 1024;
    let secret = keychain::read(CredentialLocation::Home(home))
        .await
        .ok()??;
    let credentials: Value = serde_json::from_slice(&secret).ok()?;
    let token = credentials["claudeAiOauth"]["accessToken"].as_str()?;
    if token.is_empty() {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .ok()?;
    let mut response = client
        .get("https://api.anthropic.com/api/oauth/usage?cedar_ember=1&skip_spend=1")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(
            "user-agent",
            format!("claude-cli/{cli_version} (external, cli)"),
        )
        .header("accept", "application/json")
        .header("cache-control", "no-cache")
        .send()
        .await
        .ok()?;
    if response.status() != reqwest::StatusCode::OK {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len().checked_add(chunk.len())? > BODY_LIMIT {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    parse(&serde_json::from_slice(&body).ok()?, Utc::now())
}
