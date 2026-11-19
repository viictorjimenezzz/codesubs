pub mod claude;
pub mod codex;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    AccountStatus, Error, LoginProgress, LoginRequest, Provider, ProviderCapabilities,
    ResetOutcome, Result,
};

#[derive(Clone, Copy, Debug)]
pub enum CredentialLocation<'a> {
    Default,
    Home(&'a Path),
}

#[async_trait]
pub trait ProviderBackend: Send + Sync {
    fn provider(&self) -> Provider;
    fn capabilities(&self) -> ProviderCapabilities;
    fn executable(&self) -> Option<PathBuf>;

    async fn status(&self, home: &Path, refreshing_token: bool) -> Result<AccountStatus>;
    async fn login(&self, request: LoginRequest, progress: LoginProgress) -> Result<()>;

    async fn verify_credentials(&self, home: &Path) -> Result<()> {
        self.status(home, false).await.map(|_| ())
    }

    async fn stage_credentials(
        &self,
        _source: CredentialLocation<'_>,
        _destination: &Path,
    ) -> Result<()> {
        Ok(())
    }

    async fn has_credentials(&self, _location: CredentialLocation<'_>) -> Result<bool> {
        Ok(false)
    }

    async fn remove_credentials(&self, _location: CredentialLocation<'_>) -> Result<()> {
        Ok(())
    }

    async fn redeem_reset(&self, _home: &Path, _idempotency_key: Uuid) -> Result<ResetOutcome> {
        Err(Error::Unsupported(
            "This provider does not support reset redemption through the manager.".into(),
        ))
    }
}

pub fn default_backends() -> Vec<Arc<dyn ProviderBackend>> {
    vec![
        Arc::new(codex::CodexBackend::default()),
        Arc::new(claude::ClaudeBackend::default()),
    ]
}

pub(crate) enum Executable {
    Discover,
    Explicit(Option<PathBuf>),
}

impl Executable {
    fn path(&self, provider: Provider) -> Option<PathBuf> {
        match self {
            Self::Discover => crate::runtime::locate_executable(provider),
            Self::Explicit(path) => path.clone(),
        }
    }
}

fn valid_auth_url(text: &str) -> bool {
    // The CLI can hand a URL to the OS browser. Accept HTTPS addresses only.
    text.strip_prefix("https://").is_some_and(|address| {
        let authority = address.split(['/', '?', '#']).next().unwrap_or_default();
        !authority.is_empty()
            && !authority.contains('@')
            && !text
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
    })
}

#[cfg(all(test, unix))]
mod tests;
