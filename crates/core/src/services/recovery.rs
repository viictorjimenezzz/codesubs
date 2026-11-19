use std::{fs, path::Path};

use crate::{
    CredentialLocation, Error, Provider, Result, SubscriptionManager,
    storage::{Journal, Operation, exists, move_directory, remove_directory, require_directory},
};

impl SubscriptionManager {
    pub(crate) async fn recover_pending(&self) -> Result<()> {
        self.repository.migrate_legacy()?;
        // Validate the vault before recovery can move any credentials.
        self.repository.snapshot()?;
        for id in Journal::pending(&self.repository)? {
            if let Some(_lock) = self.repository.try_account_lock(id)? {
                self.recover_locked_account(id).await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn recover_locked_account(&self, id: uuid::Uuid) -> Result<()> {
        let Some(journal) = Journal::read(&self.repository, id)? else {
            return Ok(());
        };
        let _source = match &journal.kind {
            Operation::Add { .. } | Operation::Rename { .. } | Operation::Remove => Some(
                self.repository
                    .slug_lock(journal.account.provider, &journal.account.slug)
                    .await?,
            ),
            Operation::Reconnect { .. } => None,
        };
        let _destination = match &journal.kind {
            Operation::Rename { slug } => Some(
                self.repository
                    .slug_lock(journal.account.provider, slug)
                    .await?,
            ),
            _ => None,
        };
        let _default = if matches!(journal.kind, Operation::Add { from_default: true }) {
            Some(
                self.repository
                    .default_lock(journal.account.provider)
                    .await?,
            )
        } else {
            None
        };
        self.recover_journal(&journal).await
    }

    /// Caller owns the operation's account and destination locks. The
    /// manifest is the commit marker, so crashes at any stage are repeatable.
    pub(crate) async fn recover_journal(&self, journal: &Journal) -> Result<()> {
        self.reconcile(journal).await.map_err(|error| {
            Error::RecoveryRequired(format!(
                "could not recover operation for {}: {error}",
                journal.account.reference()
            ))
        })?;
        journal.delete(&self.repository)
    }

    async fn reconcile(&self, journal: &Journal) -> Result<()> {
        let account = &journal.account;
        let provider = account.provider;
        let backend = self.backend(provider)?;
        let home = self.home(account);
        let backup = journal.backup_home(&self.repository);
        let manifest = self.repository.snapshot()?.unwrap_or_default();
        let saved = manifest
            .accounts
            .iter()
            .find(|entry| entry.id == account.id);

        // Never discard a folder or Keychain entry that is recorded as a
        // different account, including during replay of interrupted cleanup.
        if manifest.accounts.iter().any(|record| {
            record.id != account.id && journal.reserves_slug(record.provider, &record.slug)
        }) {
            return Err(Error::StaleState(account.reference()));
        }

        match &journal.kind {
            Operation::Add { from_default } => {
                if saved.is_some() {
                    if *from_default && provider == Provider::Claude {
                        backend
                            .remove_credentials(CredentialLocation::Default)
                            .await?;
                    }
                } else {
                    if *from_default
                        && provider == Provider::Codex
                        && exists(&home.join("auth.json"))?
                    {
                        self.repository.move_default_auth(&home, true)?;
                    }
                    backend
                        .remove_credentials(CredentialLocation::Home(&home))
                        .await?;
                    remove_directory(&home)?;
                }
            }
            Operation::Rename { slug } => {
                let destination = self.repository.home(provider, slug);
                if saved.is_some_and(|record| record.slug == *slug) {
                    backend
                        .remove_credentials(CredentialLocation::Home(&home))
                        .await?;
                } else {
                    if !exists(&home)? && exists(&destination)? {
                        move_directory(&destination, &home)?;
                    }
                    backend
                        .remove_credentials(CredentialLocation::Home(&destination))
                        .await?;
                }
            }
            Operation::Remove => {
                if saved.is_none() {
                    backend
                        .remove_credentials(CredentialLocation::Home(&home))
                        .await?;
                    remove_directory(&backup)?;
                } else if exists(&backup)? {
                    move_directory(&backup, &home)?;
                }
            }
            Operation::Reconnect { revision } => {
                if manifest.account_revisions.get(&account.id) == Some(revision) {
                    if exists(&backup)? {
                        merge_account_state(&backup, &home, provider)?;
                        remove_directory(&backup)?;
                    }
                } else if exists(&backup)? {
                    backend
                        .remove_credentials(CredentialLocation::Home(&home))
                        .await?;
                    backend
                        .stage_credentials(CredentialLocation::Home(&backup), &home)
                        .await?;
                    remove_directory(&home)?;
                    move_directory(&backup, &home)?;
                }
                backend
                    .remove_credentials(CredentialLocation::Home(&backup))
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn finish<T>(&self, journal: &Journal, result: Result<T>) -> Result<T> {
        self.recover_journal(journal).await?;
        result
    }
}

fn merge_account_state(source: &Path, destination: &Path, provider: Provider) -> Result<()> {
    let credentials: &[&str] = match provider {
        Provider::Codex => &[
            "auth.json",
            ".subscription-manager-links",
            ".quota-manager-links",
        ],
        Provider::Claude => &[
            ".credentials.json",
            ".claude.json",
            "credentials.json",
            ".subscription-manager-links",
            ".quota-manager-links",
        ],
    };
    merge_directory(source, destination, credentials)
}

fn merge_directory(source: &Path, destination: &Path, exclusions: &[&str]) -> Result<()> {
    require_directory(source)?;
    require_directory(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().is_some_and(|name| exclusions.contains(&name)) {
            continue;
        }

        let source = entry.path();
        let destination = destination.join(&name);
        if exists(&destination)? {
            let old = entry.file_type()?;
            let new = fs::symlink_metadata(&destination)?.file_type();
            if old.is_dir() && new.is_dir() {
                merge_directory(&source, &destination, &[])?;
                continue;
            }
            if new.is_dir() {
                remove_directory(&destination)?;
            } else if old.is_dir() {
                fs::remove_file(&destination)?;
            }
        }
        fs::rename(source, destination)?;
    }
    Ok(())
}
