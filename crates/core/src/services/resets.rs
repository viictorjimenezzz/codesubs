use std::{path::Path, time::Duration};

use uuid::Uuid;

use crate::{
    AccountRecord, Error, ProviderBackend, ResetOutcome, ResetPreview, ResetReceipt, Result,
    SubscriptionManager,
};

use super::accounts::{check_identity, interrupted, matching_account};

impl SubscriptionManager {
    pub async fn preview_reset(&self, reference: &str) -> Result<ResetPreview> {
        self.recover_pending().await?;
        let original = self.resolve(reference).await?;
        let _lock = self.repository.account_lock(original.id).await?;
        self.recover_locked_account(original.id).await?;
        let account = self.unchanged_account(&original)?;
        let backend = self.backend(account.provider)?;
        let capabilities = backend.capabilities();
        if !capabilities.read_reset_balance && !capabilities.redeem_reset {
            return Err(Error::Unsupported(format!(
                "{} reset balance is unavailable",
                account.provider
            )));
        }
        let home = self.check_managed_home(&account)?;
        let status = backend.status(&home, false).await?;
        let account = self.record_status(&account, status, self.revision(account.id)?)?;
        let now = (self.clock)();
        let quota = account.quota.as_ref();
        Ok(ResetPreview {
            available: quota
                .and_then(|quota| quota.reset_credits.as_ref())
                .map(|credits| credits.available_at(now)),
            weekly_remaining_percent: quota
                .and_then(|quota| quota.weekly())
                .map(|window| window.remaining_percent_at(now)),
            account,
            can_redeem: capabilities.redeem_reset,
        })
    }

    /// Redeem a banked reset. A pending key is persisted before contacting the
    /// provider so a later invocation can retry an interrupted request safely.
    pub async fn redeem_reset(
        &self,
        reference: &str,
        idempotency_key: Option<Uuid>,
    ) -> Result<ResetReceipt> {
        let manager = self.clone();
        let reference = reference.to_owned();
        tokio::spawn(async move { manager.redeem_inner(&reference, idempotency_key).await })
            .await
            .map_err(interrupted)?
    }

    async fn redeem_inner(
        &self,
        reference: &str,
        idempotency_key: Option<Uuid>,
    ) -> Result<ResetReceipt> {
        self.recover_pending().await?;
        let original = self.resolve(reference).await?;
        let _lock = self.repository.account_lock(original.id).await?;
        self.recover_locked_account(original.id).await?;
        let mut account = self.unchanged_account(&original)?;
        let backend = self.backend(account.provider)?;
        if !backend.capabilities().redeem_reset {
            return Err(Error::Unsupported(format!(
                "{} reset redemption is managed by the provider",
                account.provider.display_name(),
            )));
        }
        let home = self.check_managed_home(&account)?;
        let revision = self.revision(account.id)?;
        let pending = self
            .repository
            .snapshot()?
            .unwrap_or_default()
            .pending_resets
            .get(&account.id)
            .copied();
        let key = match (pending, idempotency_key) {
            (Some(pending), Some(requested)) if pending != requested => {
                return Err(Error::ResetUncertain {
                    key: pending,
                    reason: "A previous attempt is awaiting confirmation.".into(),
                });
            }
            (Some(previous), _) => previous,
            (None, Some(requested)) => requested,
            (None, None) => Uuid::new_v4(),
        };

        // A retry must reach the provider even if the first request already
        // spent the last credit. Always check that the credentials belong to
        // the account before any redemption request.
        let before = backend.status(&home, false).await?;
        check_identity(account.email.as_deref(), before.email.as_deref(), true)?;
        if pending.is_none() && idempotency_key.is_none() {
            let balance = before
                .quota
                .reset_credits
                .as_ref()
                .map(|credit| credit.available_at((self.clock)()));
            match balance {
                Some(balance) if balance > 0 => {}
                Some(_) => {
                    return Err(Error::ResetUnavailable(format!(
                        "{} has no banked resets",
                        account.reference()
                    )));
                }
                None => {
                    return Err(Error::ResetUnavailable(format!(
                        "{} reset balance is unknown",
                        account.reference()
                    )));
                }
            }
        }
        account = self.record_status(&account, before, revision)?;
        self.repository.update(|manifest| {
            matching_account(manifest, &account)?;
            manifest.pending_resets.insert(account.id, key);
            Ok(())
        })?;

        let outcome =
            match backend.redeem_reset(&home, key).await {
                Ok(outcome) => outcome,
                Err(_) => backend.redeem_reset(&home, key).await.map_err(|error| {
                    Error::ResetUncertain {
                        key,
                        reason: error.to_string(),
                    }
                })?,
            };

        // With the provider outcome confirmed, refresh and bookkeeping
        // failures must never turn the reset itself into a reported failure.
        if outcome == ResetOutcome::Reset
            && let Ok(Some(observed)) = tokio::time::timeout(
                Duration::from_secs(29),
                self.observe_reset(backend.as_ref(), &home, &account, revision),
            )
            .await
        {
            account = observed;
        }
        self.repository
            .update(|manifest| {
                manifest.pending_resets.remove(&account.id);
                Ok(())
            })
            .ok();
        if let Ok(latest) = self.unchanged_account(&account) {
            account = latest;
        }
        Ok(ResetReceipt {
            account,
            idempotency_key: key,
            outcome,
        })
    }

    async fn observe_reset(
        &self,
        backend: &dyn ProviderBackend,
        home: &Path,
        account: &AccountRecord,
        revision: Option<Uuid>,
    ) -> Option<AccountRecord> {
        let previous = account.quota.as_ref()?.weekly();
        let mut observed = None;
        for attempt in 0..10 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let status = backend.status(home, false).await.ok()?;
            let current = status.quota.weekly();
            let changed = match (previous, current) {
                (Some(before), Some(after)) => {
                    before.used_percent != after.used_percent || before.resets_at != after.resets_at
                }
                (None, Some(_)) => true,
                _ => false,
            };
            observed = Some(self.record_status(account, status, revision).ok()?);
            if changed {
                return observed;
            }
        }
        observed
    }
}
