use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AccountRecord;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "String", into = "String")]
pub enum ResetOutcome {
    Reset,
    NothingToReset,
    NoCredit,
    AlreadyRedeemed,
    Other(String),
}

impl ResetOutcome {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Reset => "reset",
            Self::NothingToReset => "nothingToReset",
            Self::NoCredit => "noCredit",
            Self::AlreadyRedeemed => "alreadyRedeemed",
            Self::Other(raw) => raw,
        }
    }
}

impl From<String> for ResetOutcome {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "reset" => Self::Reset,
            "nothingToReset" => Self::NothingToReset,
            "noCredit" => Self::NoCredit,
            "alreadyRedeemed" => Self::AlreadyRedeemed,
            _ => Self::Other(raw),
        }
    }
}

impl From<ResetOutcome> for String {
    fn from(outcome: ResetOutcome) -> Self {
        outcome.as_str().to_owned()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPreview {
    pub account: AccountRecord,
    pub available: Option<i64>,
    pub weekly_remaining_percent: Option<i64>,
    pub can_redeem: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetReceipt {
    pub account: AccountRecord,
    pub idempotency_key: Uuid,
    pub outcome: ResetOutcome,
}
