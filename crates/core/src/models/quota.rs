use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QuotaWindowKind {
    Session,
    Weekly,
    WeeklyModel,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub kind: QuotaWindowKind,
    pub label: String,
    pub used_percent: i64,
    #[serde(
        default,
        with = "super::optional_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub resets_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_duration_minutes: Option<i64>,
    /// What the window covers, such as the models sharing a group limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl QuotaWindow {
    pub fn clamped_percent(&self) -> i64 {
        self.used_percent.clamp(0, 100)
    }

    pub fn remaining_percent(&self) -> i64 {
        100 - self.clamped_percent()
    }

    pub fn remaining_percent_at(&self, date: DateTime<Utc>) -> i64 {
        if self.resets_at.is_some_and(|reset| reset <= date) {
            100
        } else {
            self.remaining_percent()
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredits {
    pub available: i64,
    #[serde(
        default,
        with = "super::optional_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_expiry: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grants: Option<Vec<ResetGrant>>,
}

impl ResetCredits {
    pub fn available_at(&self, date: DateTime<Utc>) -> i64 {
        self.grants.as_ref().map_or(self.available, |grants| {
            grants
                .iter()
                .filter(|grant| grant.is_available_at(date))
                .fold(0_i64, |total, grant| total.saturating_add(grant.available))
        })
    }

    pub fn next_expiry_at(&self, date: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.grants.as_ref().map_or(self.next_expiry, |grants| {
            grants
                .iter()
                .filter(|grant| grant.is_available_at(date))
                .filter_map(|grant| grant.expires_at)
                .min()
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetGrant {
    pub label: String,
    pub available: i64,
    #[serde(
        default,
        with = "super::optional_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub starts_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        with = "super::optional_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub expires_at: Option<DateTime<Utc>>,
    pub paused: bool,
}

impl ResetGrant {
    pub fn is_available_at(&self, date: DateTime<Utc>) -> bool {
        self.available > 0
            && !self.paused
            && self.starts_at.is_none_or(|starts| starts <= date)
            && self.expires_at.is_none_or(|expires| expires > date)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditBalance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    pub unlimited: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub windows: Vec<QuotaWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_credits: Option<ResetCredits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<CreditBalance>,
}

impl QuotaSnapshot {
    pub fn weekly(&self) -> Option<&QuotaWindow> {
        self.windows
            .iter()
            .find(|window| window.kind == QuotaWindowKind::Weekly)
            .or_else(|| {
                self.windows
                    .iter()
                    .rev()
                    .max_by_key(|window| window.window_duration_minutes.unwrap_or(0))
            })
    }

    pub fn session(&self) -> Option<&QuotaWindow> {
        let weekly = self.weekly();
        self.windows
            .iter()
            .find(|window| window.kind == QuotaWindowKind::Session && Some(*window) != weekly)
    }

    pub fn available_percent(&self, date: DateTime<Utc>) -> Option<i64> {
        self.windows
            .iter()
            .filter(|window| window.kind != QuotaWindowKind::WeeklyModel)
            .map(|window| window.remaining_percent_at(date))
            .min()
    }
}
