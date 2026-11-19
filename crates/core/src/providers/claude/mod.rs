pub mod login;
pub mod reset;
pub mod usage;

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::providers::{CredentialLocation, Executable, ProviderBackend};
use crate::runtime::{ProcessOutput, create_private_dir, run};
use crate::{
    AccountStatus, Error, LoginProgress, LoginRequest, Provider, ProviderCapabilities, Result,
};

pub struct ClaudeBackend {
    executable: Executable,
    use_system_keychain: bool,
}

impl Default for ClaudeBackend {
    fn default() -> Self {
        Self {
            executable: Executable::Discover,
            use_system_keychain: cfg!(target_os = "macos"),
        }
    }
}

impl ClaudeBackend {
    /// Uses an explicitly supplied executable without invoking the system
    /// Keychain or reset-balance probe. For system credentials, use Default.
    pub fn new(executable: Option<PathBuf>) -> Self {
        Self {
            executable: Executable::Explicit(executable),
            use_system_keychain: false,
        }
    }

    async fn command<const N: usize>(
        &self,
        home: &Path,
        arguments: [&str; N],
        timeout: u64,
        label: &str,
    ) -> Result<ProcessOutput> {
        let executable = self
            .executable()
            .ok_or(Error::ExecutableNotFound(Provider::Claude))?;
        create_private_dir(home)?;
        run(
            &executable,
            arguments,
            Some((Provider::Claude, home)),
            None,
            Duration::from_secs(timeout),
            label,
        )
        .await
    }

    async fn auth_status(&self, home: &Path) -> Result<Value> {
        let output = self
            .command(home, ["auth", "status", "--json"], 30, "Claude auth status")
            .await?;
        let payload = serde_json::from_slice::<Value>(&output.stdout).map_err(|_| {
            Error::Provider(format!(
                "Claude Code returned unreadable auth status ({}).",
                output.status
            ))
        })?;
        if payload["loggedIn"].as_bool() != Some(true) {
            return Err(Error::Provider(
                "This Claude account needs to be connected again.".into(),
            ));
        }
        if !output.status.success() {
            return Err(Error::Provider(format!(
                "Claude Code auth status failed ({}).",
                output.status
            )));
        }
        if payload["authMethod"].as_str() != Some("claude.ai")
            && payload["subscriptionType"].as_str().is_none()
        {
            return Err(Error::Provider(
                "This Claude account is not signed in with a subscription.".into(),
            ));
        }
        Ok(payload)
    }

    #[cfg(target_os = "macos")]
    async fn reset_balance(&self, home: &Path) -> Option<crate::ResetCredits> {
        if !self.use_system_keychain {
            return None;
        }
        let version = self
            .command(home, ["--version"], 10, "Claude Code version")
            .await
            .ok()?;
        if !version.status.success() {
            return None;
        }
        let version = std::str::from_utf8(&version.stdout)
            .ok()?
            .split_whitespace()
            .next()?;
        let parts: Vec<_> = version.split('.').collect();
        if parts.len() != 3
            || !parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        reset::read(home, version).await
    }
}

#[async_trait]
impl ProviderBackend for ClaudeBackend {
    fn provider(&self) -> Provider {
        Provider::Claude
    }

    fn executable(&self) -> Option<PathBuf> {
        self.executable.path(self.provider())
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            read_reset_balance: self.use_system_keychain,
            redeem_reset: false,
        }
    }

    async fn status(&self, home: &Path, _refreshing_token: bool) -> Result<AccountStatus> {
        let auth = self.auth_status(home).await?;
        // /usage is a local CLI command and does not perform a model request.
        let output = self
            .command(
                home,
                [
                    "-p",
                    "/usage",
                    "--output-format",
                    "json",
                    "--no-session-persistence",
                    "--setting-sources",
                    "project",
                ],
                90,
                "Claude /usage",
            )
            .await?;
        let usage = serde_json::from_slice::<Value>(&output.stdout).map_err(|_| {
            Error::Provider(format!(
                "Claude Code returned unreadable /usage output ({}).",
                output.status
            ))
        })?;
        if !output.status.success() || usage["is_error"].as_bool() == Some(true) {
            return Err(Error::Provider(format!(
                "Claude Code could not read subscription usage ({}).",
                output.status
            )));
        }
        let text = usage["result"].as_str().ok_or_else(|| {
            Error::Provider("Claude Code did not return a subscription usage result.".into())
        })?;
        let quota = usage::quota(text, Utc::now()).ok_or_else(|| {
            Error::Provider("Claude Code did not return quota for this account.".into())
        })?;
        #[cfg(target_os = "macos")]
        let quota = crate::QuotaSnapshot {
            reset_credits: self.reset_balance(home).await,
            ..quota
        };
        Ok(AccountStatus {
            email: auth["email"].as_str().map(str::to_owned),
            plan: auth["subscriptionType"].as_str().map(str::to_owned),
            quota,
        })
    }

    async fn verify_credentials(&self, home: &Path) -> Result<()> {
        self.auth_status(home).await.map(|_| ())
    }

    async fn login(&self, request: LoginRequest, progress: LoginProgress) -> Result<()> {
        let executable = self
            .executable()
            .ok_or(Error::ExecutableNotFound(Provider::Claude))?;
        login::run(&executable, request, progress).await
    }

    async fn stage_credentials(
        &self,
        source: CredentialLocation<'_>,
        _destination: &Path,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        if self.use_system_keychain {
            return crate::platform::keychain::stage(source, _destination).await;
        }
        match source {
            CredentialLocation::Default => Err(Error::Unsupported(
                "Adopting the default Claude login requires the macOS Keychain.".into(),
            )),
            CredentialLocation::Home(_) => Ok(()),
        }
    }

    async fn has_credentials(&self, _location: CredentialLocation<'_>) -> Result<bool> {
        #[cfg(target_os = "macos")]
        if self.use_system_keychain {
            return crate::platform::keychain::has_credentials(_location).await;
        }
        Ok(false)
    }

    async fn remove_credentials(&self, _location: CredentialLocation<'_>) -> Result<()> {
        #[cfg(target_os = "macos")]
        if self.use_system_keychain {
            return crate::platform::keychain::remove(_location).await;
        }
        Ok(())
    }
}
