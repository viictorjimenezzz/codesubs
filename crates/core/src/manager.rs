use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    AccountRecord, Error, ExecutionContext, ManagerConfig, Provider, ProviderBackend,
    ProviderCapabilities, Result, Selection, SelectionOptions, providers, runtime,
    services::selection, storage::Repository,
};

/// A reusable subscription manager. Separate instances and CLI processes
/// coordinate updates through stable locks inside the vault.
#[derive(Clone)]
pub struct Codesubs {
    pub(crate) repository: Repository,
    pub(crate) backends: Arc<Vec<Arc<dyn ProviderBackend>>>,
    pub(crate) clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl Codesubs {
    pub fn new(config: ManagerConfig) -> Result<Self> {
        Self::with_backends(config, providers::default_backends())
    }

    pub fn with_backends(
        config: ManagerConfig,
        backends: impl IntoIterator<Item = Arc<dyn ProviderBackend>>,
    ) -> Result<Self> {
        let migrate_legacy = config.migrate_legacy;
        let mut config = ManagerConfig::new(config.vault_root, config.user_home)?;
        config.migrate_legacy = migrate_legacy;
        let backends: Vec<_> = backends.into_iter().collect();
        for provider in Provider::ALL {
            if backends
                .iter()
                .filter(|backend| backend.provider() == provider)
                .count()
                > 1
            {
                return Err(Error::InvalidInput(format!(
                    "multiple backends for {provider}"
                )));
            }
        }
        Ok(Self {
            repository: Repository::new(config),
            backends: Arc::new(backends),
            clock: Arc::new(Utc::now),
        })
    }

    /// Override the observation clock when embedding the library or testing.
    pub fn with_clock(mut self, clock: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    pub fn config(&self) -> &ManagerConfig {
        self.repository.config()
    }

    pub fn home(&self, account: &AccountRecord) -> PathBuf {
        self.repository.home(account.provider, &account.slug)
    }

    /// Initialize the private vault and install its onboarding guides.
    pub async fn prepare(&self) -> Result<()> {
        self.recover_pending().await?;
        self.repository.initialize()?;
        self.repository.write_guides()
    }

    pub async fn list(&self) -> Result<Vec<AccountRecord>> {
        self.repository.migrate_legacy()?;
        Ok(self.repository.snapshot()?.unwrap_or_default().accounts)
    }

    pub async fn resolve(&self, reference: &str) -> Result<AccountRecord> {
        selection::resolve(&self.list().await?, reference)
    }

    pub async fn env(&self, reference: &str) -> Result<ExecutionContext> {
        let account = self.resolve(reference).await?;
        let provider = account.provider;
        let home = self.home(&account);
        Ok(ExecutionContext {
            account,
            set: [(
                provider.home_environment_variable().to_owned(),
                home.to_string_lossy().into_owned(),
            )]
            .into(),
            unset: provider
                .conflicting_environment_variables()
                .iter()
                .map(|value| (*value).into())
                .collect(),
            home,
        })
    }

    pub async fn pick(&self, provider: Provider, options: SelectionOptions) -> Result<Selection> {
        selection::select_account(&self.list().await?, provider, options, (self.clock)())
    }

    pub async fn submit_code(&self, code: &str, slug: Option<&str>) -> Result<()> {
        runtime::submit_login_code(&self.repository.pending(), slug, code)
    }

    pub async fn doctor(&self) -> Result<DoctorReport> {
        let manifest = self.repository.snapshot()?;
        let providers = self
            .backends
            .iter()
            .map(|backend| ProviderDiagnostic {
                provider: backend.provider(),
                executable: backend.executable(),
                capabilities: backend.capabilities(),
            })
            .collect();
        Ok(DoctorReport {
            vault_root: self.repository.root().to_owned(),
            manifest: self.repository.manifest_path(),
            manifest_version: manifest.as_ref().map(|manifest| manifest.version),
            accounts: manifest.map_or(0, |manifest| manifest.accounts.len()),
            providers,
        })
    }

    pub(crate) fn backend(&self, provider: Provider) -> Result<Arc<dyn ProviderBackend>> {
        self.backends
            .iter()
            .find(|backend| backend.provider() == provider)
            .cloned()
            .ok_or_else(|| Error::Unsupported(format!("no backend configured for {provider}")))
    }

    pub(crate) fn unchanged_account(&self, original: &AccountRecord) -> Result<AccountRecord> {
        self.repository
            .snapshot()?
            .unwrap_or_default()
            .accounts
            .into_iter()
            .find(|account| {
                account.id == original.id
                    && account.provider == original.provider
                    && account.slug == original.slug
            })
            .ok_or_else(|| Error::StaleState(original.reference()))
    }

    pub(crate) fn check_managed_home(&self, account: &AccountRecord) -> Result<PathBuf> {
        let home = self.home(account);
        crate::storage::require_directory(home.parent().unwrap_or(Path::new(".")))?;
        crate::storage::require_directory(&home)?;
        Ok(home)
    }

    pub(crate) fn revision(&self, id: Uuid) -> Result<Option<Uuid>> {
        Ok(self
            .repository
            .snapshot()?
            .unwrap_or_default()
            .account_revisions
            .get(&id)
            .copied())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub vault_root: PathBuf,
    pub manifest: PathBuf,
    pub manifest_version: Option<u32>,
    pub accounts: usize,
    pub providers: Vec<ProviderDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDiagnostic {
    pub provider: Provider,
    pub executable: Option<PathBuf>,
    pub capabilities: ProviderCapabilities,
}
