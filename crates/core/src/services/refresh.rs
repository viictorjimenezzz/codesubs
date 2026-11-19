use tokio::task::JoinSet;
use uuid::Uuid;

use crate::{
    AccountRecord, AccountStatus, Error, Provider, RefreshFailure, RefreshReport, Result,
    SubscriptionManager, storage::Manifest,
};

use super::{
    accounts::{check_identity, check_identity_available, matching_account},
    selection,
};

impl SubscriptionManager {
    /// Refresh one account or the entire vault. Successful per-account writes
    /// are committed individually and failed observations retain their cache.
    pub async fn refresh(&self, reference: Option<&str>) -> Result<RefreshReport> {
        self.recover_pending().await?;
        let manifest = self.repository.snapshot()?.unwrap_or_default();
        let accounts = match reference {
            Some(reference) => vec![selection::resolve(&manifest.accounts, reference)?],
            None => manifest.accounts.clone(),
        };
        self.refresh_accounts(accounts, manifest).await
    }

    pub async fn refresh_provider(&self, provider: Provider) -> Result<RefreshReport> {
        self.recover_pending().await?;
        let manifest = self.repository.snapshot()?.unwrap_or_default();
        let accounts = manifest
            .accounts
            .iter()
            .filter(|account| account.provider == provider)
            .cloned()
            .collect();
        self.refresh_accounts(accounts, manifest).await
    }

    async fn refresh_accounts(
        &self,
        accounts: Vec<AccountRecord>,
        manifest: Manifest,
    ) -> Result<RefreshReport> {
        let mut tasks = JoinSet::new();
        let total = accounts.len();
        for (index, account) in accounts.into_iter().enumerate() {
            let manager = self.clone();
            let revision = manifest.account_revisions.get(&account.id).copied();
            tasks.spawn(async move {
                let result = manager.refresh_one(&account, revision).await;
                (index, account.reference(), result)
            });
        }
        let mut results = Vec::with_capacity(total);
        while let Some(task) = tasks.join_next().await {
            results.push(task.map_err(super::accounts::interrupted)?);
        }
        results.sort_by_key(|(index, _, _)| *index);
        let mut report = RefreshReport {
            accounts: Vec::new(),
            failures: Vec::new(),
        };
        for (_, reference, result) in results {
            match result {
                Ok(account) => report.accounts.push(account),
                Err(error) => report.failures.push(RefreshFailure {
                    reference,
                    code: error.code().to_owned(),
                    message: error.to_string(),
                }),
            }
        }
        Ok(report)
    }

    async fn refresh_one(
        &self,
        original: &AccountRecord,
        revision: Option<Uuid>,
    ) -> Result<AccountRecord> {
        let lock = self.repository.account_lock(original.id).await?;
        self.recover_locked_account(original.id).await?;
        let account = self.unchanged_account(original)?;
        let current_revision = self.revision(account.id)?;
        if current_revision != revision {
            return Err(Error::StaleState(account.reference()));
        }
        let backend = self.backend(account.provider)?;
        let home = self.check_managed_home(&account)?;
        self.repository.link_shared(&account)?;
        let status = backend.status(&home, false).await?;
        let observed = self.record_status(&account, status, current_revision)?;
        drop(lock);
        if observed.slug.starts_with("new-")
            && original.email.is_none()
            && let Some(email) = &observed.email
        {
            return self
                .rename_generated(observed.clone(), &crate::storage::slug_from_email(email))
                .await
                .or_else(|error| {
                    if matches!(error, Error::RecoveryRequired(_)) {
                        Err(error)
                    } else {
                        Ok(observed)
                    }
                });
        }
        Ok(observed)
    }

    pub(super) fn record_status(
        &self,
        original: &AccountRecord,
        status: AccountStatus,
        revision: Option<Uuid>,
    ) -> Result<AccountRecord> {
        check_identity(original.email.as_deref(), status.email.as_deref(), false)?;
        self.repository.update(|manifest| {
            if manifest.account_revisions.get(&original.id).copied() != revision {
                return Err(Error::StaleState(original.reference()));
            }
            check_identity_available(
                manifest,
                original.provider,
                status.email.as_deref(),
                Some(original.id),
            )?;
            let current = matching_account(manifest, original)?;
            self.apply_status(current, status);
            Ok(current.clone())
        })
    }
}
