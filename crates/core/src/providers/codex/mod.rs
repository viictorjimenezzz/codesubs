pub mod payload;
mod session;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::providers::{Executable, ProviderBackend};
use crate::runtime::{create_private_dir, wait_for_cancel};
use crate::{
    AccountStatus, Error, LoginEvent, LoginProgress, LoginRequest, Provider, ProviderCapabilities,
    ResetOutcome, Result,
};

use session::Session;

pub struct CodexBackend {
    executable: Executable,
}

impl Default for CodexBackend {
    fn default() -> Self {
        Self {
            executable: Executable::Discover,
        }
    }
}

impl CodexBackend {
    pub fn new(executable: Option<PathBuf>) -> Self {
        Self {
            executable: Executable::Explicit(executable),
        }
    }

    async fn session(&self, home: &Path) -> Result<Session> {
        let mut session = self.spawn(home)?;
        if let Err(error) = session.initialize().await {
            session.stop().await;
            return Err(error);
        }
        Ok(session)
    }

    fn spawn(&self, home: &Path) -> Result<Session> {
        let executable = self
            .executable()
            .ok_or(Error::ExecutableNotFound(Provider::Codex))?;
        create_private_dir(home)?;
        write_config(home)?;
        Session::spawn(&executable, home)
    }
}

#[async_trait]
impl ProviderBackend for CodexBackend {
    fn provider(&self) -> Provider {
        Provider::Codex
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            read_reset_balance: true,
            redeem_reset: true,
        }
    }

    fn executable(&self) -> Option<PathBuf> {
        self.executable.path(self.provider())
    }

    async fn status(&self, home: &Path, refreshing_token: bool) -> Result<AccountStatus> {
        let mut session = self.session(home).await?;
        let result = async {
            let mut identity =
                payload::account(&read_account(&mut session, refreshing_token).await?);
            if identity.is_none() && refreshing_token {
                identity = payload::account(&read_account(&mut session, false).await?);
            }
            let (email, plan) = identity.ok_or_else(|| {
                Error::Provider("This Codex account needs to be connected again.".into())
            })?;
            let quota = session
                .request("account/rateLimits/read", json!({}))
                .await?;
            let quota = payload::quota(&quota).ok_or_else(|| {
                Error::Provider("Codex did not return quota for this account.".into())
            })?;
            Ok(AccountStatus { email, plan, quota })
        }
        .await;
        session.stop().await;
        result
    }

    async fn verify_credentials(&self, home: &Path) -> Result<()> {
        let mut session = self.session(home).await?;
        let identity = read_account(&mut session, false).await;
        session.stop().await;
        payload::account(&identity?)
            .map(|_| ())
            .ok_or_else(|| Error::Provider("Codex did not accept the moved login.".into()))
    }

    async fn login(&self, mut request: LoginRequest, progress: LoginProgress) -> Result<()> {
        if request
            .cancel
            .as_ref()
            .is_some_and(|cancel| *cancel.borrow())
        {
            return Err(Error::Cancelled);
        }
        let mut session = self.spawn(&request.home)?;
        let result = async {
            tokio::select! {
                biased;
                _ = wait_for_cancel(&mut request.cancel) => return Err(Error::Cancelled),
                result = session.initialize() => result?,
            }
            let started = tokio::select! {
                biased;
                _ = wait_for_cancel(&mut request.cancel) => return Err(Error::Cancelled),
                result = session.request("account/login/start", json!({
                    "type": "chatgpt",
                    "appBrand": "codex",
                    "useHostedLoginSuccessPage": true,
                    "codexStreamlinedLogin": true,
                })) => result?,
            };
            let id = started["loginId"].as_str().filter(|id| !id.is_empty())
                .ok_or_else(|| Error::Login("Codex did not return a sign-in ID.".into()))?;
            let url = started["authUrl"].as_str().filter(|url| super::valid_auth_url(url))
                .ok_or_else(|| Error::Login("Codex did not return a valid sign-in URL.".into()))?;
            progress(LoginEvent::SignInUrl { url: url.to_owned() });
            let result = tokio::select! {
                biased;
                _ = wait_for_cancel(&mut request.cancel) => Err(Error::Cancelled),
                report = tokio::time::timeout(Duration::from_secs(600), session.wait_for_login(id)) => {
                    match report {
                        Ok(Ok(true)) => Ok(()),
                        Ok(Ok(false)) => Err(Error::Login("Codex sign-in did not complete.".into())),
                        Ok(Err(error)) => Err(error),
                        Err(_) => Err(Error::Timeout("Codex browser sign-in".into())),
                    }
                }
            };
            if matches!(result, Err(Error::Cancelled | Error::Timeout(_))) {
                session.cancel_login(id).await;
            }
            result
        }.await;
        session.stop().await;
        result
    }

    async fn redeem_reset(&self, home: &Path, idempotency_key: Uuid) -> Result<ResetOutcome> {
        let mut session = self.session(home).await?;
        let response = session
            .request(
                "account/rateLimitResetCredit/consume",
                json!({"idempotencyKey": idempotency_key.to_string()}),
            )
            .await;
        session.stop().await;
        payload::reset_outcome(&response?)
            .ok_or_else(|| Error::Provider("Codex returned an unreadable reset outcome.".into()))
    }
}

async fn read_account(session: &mut Session, refreshing_token: bool) -> Result<Value> {
    session
        .request("account/read", json!({"refreshToken": refreshing_token}))
        .await
}

fn write_config(home: &Path) -> io::Result<()> {
    let path = home.join("config.toml");
    if std::fs::symlink_metadata(&path).is_ok() {
        return Ok(());
    }
    let mut config = tempfile::NamedTempFile::new_in(home)?;
    config.write_all(b"cli_auth_credentials_store = \"file\"\n\n[analytics]\nenabled = false\n")?;
    match config.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}
