use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, QuotaSnapshot, Result};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
    Antigravity,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::Antigravity];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Antigravity => "antigravity",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Antigravity => "Antigravity",
        }
    }

    /// The variable that points the provider CLI at an account folder.
    /// Antigravity has none: its active account is a symlink under
    /// `~/.gemini`, switched by `Codesubs::activate`.
    pub const fn home_environment_variable(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some("CODEX_HOME"),
            Self::Claude => Some("CLAUDE_CONFIG_DIR"),
            Self::Antigravity => None,
        }
    }

    pub const fn conflicting_environment_variables(self) -> &'static [&'static str] {
        match self {
            Self::Codex | Self::Antigravity => &[],
            Self::Claude => &[
                "CLAUDE_CODE_OAUTH_TOKEN",
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
            ],
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Provider {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "antigravity" => Ok(Self::Antigravity),
            _ => Err(Error::InvalidInput(format!("unknown provider {value:?}"))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub id: Uuid,
    pub provider: Provider,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<QuotaSnapshot>,
    #[serde(
        default,
        with = "super::optional_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_updated: Option<DateTime<Utc>>,
}

impl AccountRecord {
    pub fn new(provider: Provider, slug: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            provider,
            slug: slug.into(),
            email: None,
            plan: None,
            quota: None,
            last_updated: None,
        }
    }

    pub fn reference(&self) -> String {
        format!("{}/{}", self.provider, self.slug)
    }

    pub fn is_provisional(&self) -> bool {
        self.slug.starts_with("new-")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountStatus {
    pub email: Option<String>,
    pub plan: Option<String>,
    pub quota: QuotaSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionContext {
    pub account: AccountRecord,
    pub home: PathBuf,
    pub set: BTreeMap<String, String>,
    pub unset: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct SelectionOptions {
    pub minimum_percent: i64,
    pub max_age: Duration,
    pub require_fresh: bool,
}

impl Default for SelectionOptions {
    fn default() -> Self {
        Self {
            minimum_percent: 1,
            max_age: Duration::from_secs(3600),
            require_fresh: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub account: AccountRecord,
    pub available_percent: i64,
    pub freshness: Freshness,
    pub projected: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    pub accounts: Vec<AccountRecord>,
    pub failures: Vec<RefreshFailure>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshFailure {
    pub reference: String,
    pub code: String,
    pub message: String,
}

/// Provider-specific reset operations implemented by the configured backend.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub read_reset_balance: bool,
    pub redeem_reset: bool,
}
