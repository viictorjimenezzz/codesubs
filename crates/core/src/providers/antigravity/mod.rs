//! Antigravity (`agy`) accounts.
//!
//! DETAILS >>> The CLI keeps its OAuth token in
//! `$HOME/.gemini/antigravity-cli/antigravity-oauth-token` and has no setting
//! to move it. Each account stores its token in its vault folder. Two links
//! point at it:
//!
//! - a private probe home (`<account>/home`) that quota reads run under, so any
//!   account can be read without switching;
//! - the real `~/.gemini` token, repointed by [`activate`] when an account is
//!   chosen for an interactive session.
//!
//! The CLI may replace a linked token with a regular file when it refreshes.
//! That file is moved back into the vault before the link is used again.

pub mod login;
pub mod usage;

use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::providers::{CredentialLocation, Executable, ProviderBackend};
use crate::runtime::{create_private_dir, run};
use crate::{
    AccountStatus, Error, LoginProgress, LoginRequest, Provider, ProviderCapabilities, Result,
};

pub const TOKEN: &str = "antigravity-oauth-token";
const TOKEN_DIRECTORY: &str = ".gemini/antigravity-cli";
/// From the probe token's folder up to the account folder.
const VAULT_TOKEN_FROM_PROBE: &str = "../../../antigravity-oauth-token";
const USAGE_TIMEOUT: Duration = Duration::from_secs(90);

pub fn probe_home(home: &Path) -> PathBuf {
    home.join("home")
}

pub(crate) fn probe_token(home: &Path) -> PathBuf {
    probe_home(home).join(TOKEN_DIRECTORY).join(TOKEN)
}

fn vault_token(home: &Path) -> PathBuf {
    home.join(TOKEN)
}

/// Makes the probe home's token a link to the vault token, first moving a
/// token the CLI wrote there as a regular file into the vault.
pub(crate) fn link_probe(home: &Path) -> io::Result<()> {
    let link = probe_token(home);
    create_private_dir(link.parent().expect("probe token has a parent"))?;
    if fs::symlink_metadata(&link).is_ok_and(|meta| meta.is_file()) {
        fs::rename(&link, vault_token(home))?;
    }
    if !vault_token(home).try_exists()? {
        return Ok(());
    }
    match fs::read_link(&link) {
        Ok(target) if target == Path::new(VAULT_TOKEN_FROM_PROBE) => Ok(()),
        _ => {
            let _ = fs::remove_file(&link);
            symlink(VAULT_TOKEN_FROM_PROBE, &link)
        }
    }
}

fn active_marker(home: &Path) -> PathBuf {
    home.parent()
        .expect("account folder has a parent")
        .join(".active")
}

fn active_link(user_home: &Path) -> PathBuf {
    user_home.join(TOKEN_DIRECTORY).join(TOKEN)
}

/// Points `~/.gemini`'s token at this account, so a plain `agy` runs as it.
/// The caller holds the provider's default-home lock.
pub fn activate(home: &Path, user_home: &Path) -> Result<()> {
    let token = vault_token(home);
    if !token.try_exists()? {
        return Err(Error::Provider(
            "This Antigravity account has no saved login. Reconnect it.".into(),
        ));
    }
    let link = active_link(user_home);
    create_private_dir(link.parent().expect("active token has a parent"))?;
    let marker = active_marker(home);
    match fs::symlink_metadata(&link) {
        Ok(meta) if meta.is_file() => {
            // The CLI refreshed the previous account's token over our link.
            let previous = fs::read_to_string(&marker)
                .map(PathBuf::from)
                .ok()
                .filter(|path| path.parent().is_some_and(Path::is_dir));
            let Some(previous) = previous else {
                return Err(Error::Unsupported(format!(
                    "Antigravity is signed in outside codesubs ({}). Remove that file first.",
                    link.display()
                )));
            };
            fs::rename(&link, previous)?;
        }
        Ok(_) => fs::remove_file(&link)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    symlink(&token, &link)?;
    fs::write(&marker, token.to_string_lossy().as_bytes())?;
    Ok(())
}

/// Stops pointing `~/.gemini` at an account that is being removed.
fn deactivate(home: &Path, user_home: &Path) {
    let link = active_link(user_home);
    if fs::read_link(&link).is_ok_and(|target| target == vault_token(home)) {
        let _ = fs::remove_file(&link);
        let _ = fs::remove_file(active_marker(home));
    }
}

fn token_email(home: &Path) -> Result<String> {
    let reconnect =
        || Error::Provider("This Antigravity account needs to be connected again.".into());
    let token: Value =
        serde_json::from_slice(&fs::read(vault_token(home)).map_err(|_| reconnect())?)
            .map_err(|_| reconnect())?;
    token["id_token_email"]
        .as_str()
        .map(str::to_lowercase)
        .ok_or_else(reconnect)
}

pub struct AntigravityBackend {
    executable: Executable,
}

impl Default for AntigravityBackend {
    fn default() -> Self {
        Self {
            executable: Executable::Discover,
        }
    }
}

impl AntigravityBackend {
    /// Uses an explicitly supplied executable.
    pub fn new(executable: Option<PathBuf>) -> Self {
        Self {
            executable: Executable::Explicit(executable),
        }
    }
}

#[async_trait]
impl ProviderBackend for AntigravityBackend {
    fn provider(&self) -> Provider {
        Provider::Antigravity
    }

    fn executable(&self) -> Option<PathBuf> {
        self.executable.path(self.provider())
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    async fn status(&self, home: &Path, _refreshing_token: bool) -> Result<AccountStatus> {
        let executable = self
            .executable()
            .ok_or(Error::ExecutableNotFound(Provider::Antigravity))?;
        let email = token_email(home)?;
        link_probe(home)?;
        // /usage is answered by the CLI without a model request.
        let output = run(
            &executable,
            ["-p", "/usage", "--output-format", "json"],
            Some((Provider::Antigravity, home)),
            None,
            USAGE_TIMEOUT,
            "Antigravity /usage",
        )
        .await?;
        link_probe(home)?;
        Ok(AccountStatus {
            email: Some(email),
            plan: None,
            quota: usage::quota(&output.stdout)?,
        })
    }

    async fn login(&self, request: LoginRequest, progress: LoginProgress) -> Result<()> {
        let executable = self
            .executable()
            .ok_or(Error::ExecutableNotFound(Provider::Antigravity))?;
        let home = request.home.clone();
        let result = login::run(&executable, request, progress).await;
        link_probe(&home)?;
        result
    }

    async fn remove_credentials(&self, location: CredentialLocation<'_>) -> Result<()> {
        if let (CredentialLocation::Home(home), Some(user_home)) = (location, std::env::home_dir())
        {
            deactivate(home, &user_home);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(root: &Path, slug: &str) -> PathBuf {
        let home = root.join("vault/antigravity").join(slug);
        fs::create_dir_all(&home).unwrap();
        fs::write(
            vault_token(&home),
            format!("{{\"token\":\"{slug}\",\"id_token_email\":\"A@Example.com\"}}"),
        )
        .unwrap();
        home
    }

    #[test]
    fn probe_link_resolves_to_the_vault_token_and_survives_a_refresh_rewrite() {
        let root = tempfile::tempdir().unwrap();
        let home = account(root.path(), "one");
        link_probe(&home).unwrap();
        assert_eq!(
            fs::read_to_string(probe_token(&home)).unwrap(),
            fs::read_to_string(vault_token(&home)).unwrap()
        );
        // The CLI replaces the link with a refreshed regular file.
        fs::remove_file(probe_token(&home)).unwrap();
        fs::write(probe_token(&home), "refreshed").unwrap();
        link_probe(&home).unwrap();
        assert_eq!(fs::read_to_string(vault_token(&home)).unwrap(), "refreshed");
        assert!(
            fs::symlink_metadata(probe_token(&home))
                .unwrap()
                .is_symlink()
        );
        assert!(token_email(&home).is_err(), "refreshed text is not a token");
    }

    #[test]
    fn activation_switches_accounts_and_keeps_a_refreshed_token() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let (one, two) = (account(root.path(), "one"), account(root.path(), "two"));
        activate(&one, &user).unwrap();
        assert_eq!(
            fs::read_link(active_link(&user)).unwrap(),
            vault_token(&one)
        );
        // The CLI refreshes account one's token over the link.
        fs::remove_file(active_link(&user)).unwrap();
        fs::write(active_link(&user), "refreshed-one").unwrap();
        activate(&two, &user).unwrap();
        assert_eq!(
            fs::read_to_string(vault_token(&one)).unwrap(),
            "refreshed-one"
        );
        assert_eq!(
            fs::read_link(active_link(&user)).unwrap(),
            vault_token(&two)
        );
        deactivate(&two, &user);
        assert!(!active_link(&user).exists());
    }

    #[test]
    fn an_unmanaged_login_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let home = account(root.path(), "one");
        fs::create_dir_all(active_link(&user).parent().unwrap()).unwrap();
        fs::write(active_link(&user), "someone-elses").unwrap();
        assert!(activate(&home, &user).is_err());
        assert_eq!(
            fs::read_to_string(active_link(&user)).unwrap(),
            "someone-elses"
        );
    }

    #[test]
    fn email_comes_from_the_saved_token_in_lowercase() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            token_email(&account(root.path(), "one")).unwrap(),
            "a@example.com"
        );
    }
}
