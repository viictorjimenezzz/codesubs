use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{AccountRecord, Error, GatewayRecord, ManagerConfig, Provider, Result};

use super::{Manifest, homes, locks, shared_config};

#[derive(Clone)]
pub(crate) struct Repository {
    config: ManagerConfig,
}

impl Repository {
    pub(crate) fn new(config: ManagerConfig) -> Self {
        Self { config }
    }

    pub(crate) fn config(&self) -> &ManagerConfig {
        &self.config
    }

    pub(crate) fn root(&self) -> &Path {
        &self.config.vault_root
    }

    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.root().join("accounts.json")
    }

    pub(crate) fn gateways_path(&self) -> PathBuf {
        self.root().join("gateways.json")
    }

    /// Where an API-key account keeps an agent's configuration folder.
    pub(crate) fn gateway_home(&self, gateway: &GatewayRecord, agent: Provider) -> PathBuf {
        self.root()
            .join("gateways")
            .join(format!("{}-{}", gateway.service, gateway.slug))
            .join(agent.as_str())
    }

    pub(crate) fn home(&self, provider: Provider, slug: &str) -> PathBuf {
        self.root().join(provider.as_str()).join(slug)
    }

    pub(crate) fn default_home(&self, provider: Provider) -> PathBuf {
        self.config.user_home.join(format!(".{provider}"))
    }

    pub(crate) fn pending(&self) -> PathBuf {
        self.root().join(".pending")
    }

    pub(crate) fn initialize(&self) -> Result<()> {
        homes::private_directory(self.root())?;
        homes::private_directory(&self.root().join(".locks"))?;
        for provider in Provider::ALL {
            homes::private_directory(&self.root().join(provider.as_str()))?;
        }
        homes::private_directory(&self.pending())?;
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Result<Option<Manifest>> {
        if !homes::exists(self.root())? {
            return Ok(None);
        }
        homes::require_directory(self.root())?;
        self.read_unlocked()
    }

    fn read_unlocked(&self) -> Result<Option<Manifest>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.manifest_path())
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !file.metadata()?.is_file() {
            return Err(Error::CorruptManifest(
                "expected a regular manifest file".into(),
            ));
        }
        Manifest::read(file, self).map(Some)
    }

    fn manifest_lock(&self) -> Result<File> {
        locks::lock(&self.root().join(".locks/manifest"))
    }

    pub(crate) fn update<T>(&self, update: impl FnOnce(&mut Manifest) -> Result<T>) -> Result<T> {
        self.initialize()?;
        let _lock = self.manifest_lock()?;
        let mut manifest = self.read_unlocked()?.unwrap_or_default();
        let value = update(&mut manifest)?;
        manifest.write(self)?;
        Ok(value)
    }

    pub(crate) fn gateways(&self) -> Result<Vec<GatewayRecord>> {
        if !homes::exists(&self.gateways_path())? {
            return Ok(Vec::new());
        }
        let raw = fs::read(self.gateways_path())?;
        serde_json::from_slice(&raw)
            .map_err(|error| Error::CorruptManifest(format!("gateways.json: {error}")))
    }

    /// API-key accounts live beside the version 3 account manifest, so
    /// subscription tooling that predates them never rewrites them.
    pub(crate) fn update_gateways<T>(
        &self,
        update: impl FnOnce(&mut Vec<GatewayRecord>) -> Result<T>,
    ) -> Result<T> {
        self.initialize()?;
        let _lock = locks::lock(&self.root().join(".locks/gateways"))?;
        let mut gateways = self.gateways()?;
        let value = update(&mut gateways)?;
        super::manifest::write_json(&self.gateways_path(), &gateways)?;
        Ok(value)
    }

    /// Creates an agent folder for an API-key account that shares the user's
    /// settings, skills and instructions like subscription accounts do.
    pub(crate) fn prepare_gateway_home(&self, home: &Path, agent: Provider) -> Result<()> {
        homes::private_directory(home.parent().expect("gateway home has a parent"))?;
        homes::private_directory(home)?;
        shared_config::link(agent, home, &self.default_home(agent))
    }

    pub(crate) async fn account_lock(&self, id: Uuid) -> Result<File> {
        self.initialize()?;
        locks::lock_async(&self.root().join(".locks").join(format!("account-{id}"))).await
    }

    pub(crate) fn try_account_lock(&self, id: Uuid) -> Result<Option<File>> {
        self.initialize()?;
        locks::try_lock(&self.root().join(".locks").join(format!("account-{id}")))
    }

    pub(crate) async fn slug_lock(&self, provider: Provider, slug: &str) -> Result<File> {
        self.initialize()?;
        locks::lock_async(
            &self
                .root()
                .join(".locks")
                .join(format!("slug-{provider}-{:x}", Sha256::digest(slug))),
        )
        .await
    }

    pub(crate) async fn default_lock(&self, provider: Provider) -> Result<File> {
        self.initialize()?;
        locks::lock_async(
            &self
                .root()
                .join(".locks")
                .join(format!("default-{provider}")),
        )
        .await
    }

    pub(crate) fn prepare_home(&self, account: &AccountRecord) -> Result<()> {
        let home = self.home(account.provider, &account.slug);
        homes::create_account_directory(&home)?;
        if let Err(error) = self.link_shared(account) {
            homes::remove_directory(&home)?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn prepare_reconnect_home(
        &self,
        account: &AccountRecord,
        backup: &Path,
    ) -> Result<()> {
        let home = self.home(account.provider, &account.slug);
        homes::create_account_directory(&home)?;
        let ledger = backup.join(shared_config::LINK_LEDGER);
        if homes::exists(&ledger)? && fs::symlink_metadata(&ledger)?.is_file() {
            fs::copy(ledger, home.join(shared_config::LINK_LEDGER))?;
        }
        self.link_shared(account)
    }

    pub(crate) fn link_shared(&self, account: &AccountRecord) -> Result<()> {
        let home = self.home(account.provider, &account.slug);
        homes::require_directory(&home)?;
        shared_config::link(
            account.provider,
            &home,
            &self.default_home(account.provider),
        )
    }

    pub(crate) fn move_default_auth(&self, home: &Path, restore: bool) -> Result<()> {
        let default_home = self.default_home(Provider::Codex);
        let default = default_home.join("auth.json");
        let staged = home.join("auth.json");
        let (source, destination) = if restore {
            (&staged, &default)
        } else {
            (&default, &staged)
        };
        if !fs::symlink_metadata(source)?.is_file() {
            return Err(Error::InvalidInput(format!(
                "expected a regular Codex login at {}",
                source.display()
            )));
        }
        homes::move_without_replacing(source, destination)
    }

    pub(crate) fn write_guides(&self) -> Result<()> {
        write_text(&self.root().join("README.md"), GUIDE)?;
        write_text(&self.root().join("ADD.md"), ADD_GUIDE)
    }
}

pub(crate) fn write_text(path: &Path, text: &str) -> Result<()> {
    use io::Write;
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
        && fs::read(path).is_ok_and(|contents| contents == text.as_bytes())
    {
        return Ok(());
    }
    let mut stage = tempfile::NamedTempFile::new_in(path.parent().expect("guide has a parent"))?;
    stage.write_all(text.as_bytes())?;
    stage.persist(path).map_err(|error| error.error)?;
    Ok(())
}

const GUIDE: &str = "# codesubs credentials

Use the standalone codesubs command with the app open or closed.

    codesubs list
    codesubs refresh
    codesubs pick claude
    codesubs env claude/work
    codesubs exec codex/work -- codex exec '…'
    codesubs exec antigravity/me-gmail -- agy    # switches ~/.gemini's token
    codesubs exec melious/default -- claude      # API-key account

Account references use provider/slug. accounts.json records exact provider home
paths. Use the path returned by the CLI for Claude Code. Its Keychain entry
depends on that exact configuration-directory string.

Use the manager to add, reconnect, rename, and remove accounts. Keep each
provider login in its own account home. Provider token rotation can invalidate
duplicate credentials. See ADD.md for onboarding.
";

const ADD_GUIDE: &str = "# Add a subscription account

Run codesubs add with the provider and expected account email.

    codesubs add codex --email someone@example.com
    codesubs add claude --email someone@example.com
    codesubs add antigravity --email someone@gmail.com

Open the sign-in URL using the intended account. Keep the originating add
command running. If Claude shows an authorization code, send it from another
terminal using:

    codesubs add-code '<authorization code>'

Use --slug to identify a pending login when several are running. The add
command completes after it verifies the logged-in identity. Confirm the account
with codesubs list.

Use --from-default only when explicitly moving an active default login into
the vault. Sign-in to managed accounts should remain isolated from provider
sessions using the default home.

# Add an API-key account

An API account runs Claude Code or Codex against a compatible service such as
Melious with a stored key. It has no sign-in or quota. Pipe the key in so it
stays out of argv and shell history; it is kept in the macOS Keychain.

    printf '%s' \"$KEY\" | codesubs api add melious --key-stdin
    codesubs exec melious/default -- claude
    codesubs exec melious/default --agent codex -- codex

Other services need their endpoints and models (--anthropic-url for Claude
Code, --openai-url for Codex, --big, --small). Change models later with
codesubs api set-models. codesubs api check confirms the key and both models.
";
