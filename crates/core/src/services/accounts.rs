use std::{fs, path::Path};

use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    AccountRecord, AccountStatus, AddOptions, CredentialLocation, Error, LoginProgress,
    LoginRequest, Provider, Result, SubscriptionManager,
    storage::{
        Journal, Manifest, Operation, exists, move_directory, slug_from_email, validate_slug,
    },
};

impl SubscriptionManager {
    /// Sign in with a provider CLI, verify identity and register the account.
    pub async fn add(
        &self,
        provider: Provider,
        options: AddOptions,
        progress: LoginProgress,
    ) -> Result<AccountRecord> {
        let manager = self.clone();
        // Let the transaction finish if the caller drops its waiting future.
        // Explicit login cancellation still runs through the rollback path.
        tokio::spawn(async move { manager.add_inner(provider, options, progress).await })
            .await
            .map_err(interrupted)?
    }

    async fn add_inner(
        &self,
        provider: Provider,
        options: AddOptions,
        progress: LoginProgress,
    ) -> Result<AccountRecord> {
        self.recover_pending().await?;
        let backend = self.backend(provider)?;
        let expected = options
            .expected_email
            .as_deref()
            .map(str::trim)
            .map(str::to_lowercase);
        if expected.as_deref() == Some("") {
            return Err(Error::InvalidInput(
                "expected email must not be empty".into(),
            ));
        }
        check_identity_available(
            &self.repository.snapshot()?.unwrap_or_default(),
            provider,
            expected.as_deref(),
            None,
        )?;

        let provisional = options.slug.is_none() && expected.is_none();
        let mut account = AccountRecord::new(provider, "");
        let initial_slug = match &options.slug {
            Some(slug) => {
                validate_slug(slug)?;
                slug.clone()
            }
            None => expected
                .as_deref()
                .map(slug_from_email)
                .unwrap_or_else(|| format!("new-{}", &account.id.simple().to_string()[..8])),
        };
        let account_lock = self.repository.account_lock(account.id).await?;
        let (slug, slug_lock) = self
            .reserve_slug(provider, &initial_slug, options.slug.is_some())
            .await?;
        account.slug = slug;

        let home = self.home(&account);
        if backend
            .has_credentials(CredentialLocation::Home(&home))
            .await?
        {
            return Err(Error::SlugTaken(account.reference()));
        }

        let default_lock = if options.from_default {
            Some(self.repository.default_lock(provider).await?)
        } else {
            None
        };

        let journal = Journal {
            account: account.clone(),
            kind: Operation::Add {
                from_default: options.from_default,
            },
        };
        journal.save(&self.repository)?;
        let work = async {
            check_cancelled(options.cancel.as_ref())?;
            self.repository.prepare_home(&account)?;
            if options.from_default {
                match provider {
                    Provider::Codex => self.repository.move_default_auth(&home, false)?,
                    Provider::Claude => {
                        backend
                            .stage_credentials(CredentialLocation::Default, &home)
                            .await?
                    }
                }
            } else {
                backend
                    .login(
                        LoginRequest {
                            home: home.clone(),
                            expected_email: expected.clone(),
                            pending_directory: self.repository.pending(),
                            slug: account.slug.clone(),
                            accept_terminal_input: options.accept_terminal_input,
                            cancel: options.cancel.clone(),
                        },
                        progress,
                    )
                    .await?;
            }
            let status = backend.status(&home, false).await?;
            check_cancelled(options.cancel.as_ref())?;
            check_identity(expected.as_deref(), status.email.as_deref(), true)?;
            self.apply_status(&mut account, status);
            self.repository.update(|manifest| {
                check_identity_available(manifest, provider, account.email.as_deref(), None)?;
                if manifest
                    .accounts
                    .iter()
                    .any(|entry| entry.provider == provider && entry.slug == account.slug)
                {
                    return Err(Error::SlugTaken(account.reference()));
                }
                manifest.mark_changed(account.id);
                manifest.accounts.push(account.clone());
                Ok(())
            })
        }
        .await;

        self.finish(&journal, work).await?;
        self.repository.write_guides().ok();
        drop((account_lock, slug_lock, default_lock));

        if provisional && let Some(email) = &account.email {
            // The account already has a usable, verified home. A clash or
            // verification failure during the cosmetic rename retains it.
            return self
                .rename_generated(account.clone(), &slug_from_email(email))
                .await
                .or_else(|error| {
                    if matches!(error, Error::RecoveryRequired(_)) {
                        Err(error)
                    } else {
                        Ok(account)
                    }
                });
        }
        Ok(account)
    }

    pub async fn reconnect(
        &self,
        reference: &str,
        progress: LoginProgress,
        accept_terminal_input: bool,
    ) -> Result<AccountRecord> {
        self.reconnect_with_cancellation(reference, progress, accept_terminal_input, None)
            .await
    }

    pub async fn reconnect_with_cancellation(
        &self,
        reference: &str,
        progress: LoginProgress,
        accept_terminal_input: bool,
        cancel: Option<watch::Receiver<bool>>,
    ) -> Result<AccountRecord> {
        let manager = self.clone();
        let reference = reference.to_owned();
        tokio::spawn(async move {
            manager
                .reconnect_inner(&reference, progress, accept_terminal_input, cancel)
                .await
        })
        .await
        .map_err(interrupted)?
    }

    async fn reconnect_inner(
        &self,
        reference: &str,
        progress: LoginProgress,
        accept_terminal_input: bool,
        cancel: Option<watch::Receiver<bool>>,
    ) -> Result<AccountRecord> {
        self.recover_pending().await?;
        let original = self.resolve(reference).await?;
        let _lock = self.repository.account_lock(original.id).await?;
        self.recover_locked_account(original.id).await?;
        let mut account = self.unchanged_account(&original)?;
        let home = self.check_managed_home(&account)?;
        let backend = self.backend(account.provider)?;
        let revision = Uuid::new_v4();
        let journal = Journal {
            account: account.clone(),
            kind: Operation::Reconnect { revision },
        };
        let backup = journal.backup_home(&self.repository);
        self.ensure_backup_available(&backup, account.provider)
            .await?;
        journal.save(&self.repository)?;

        let result = async {
            check_cancelled(cancel.as_ref())?;
            backend
                .stage_credentials(CredentialLocation::Home(&home), &backup)
                .await?;
            move_directory(&home, &backup)?;
            backend
                .remove_credentials(CredentialLocation::Home(&home))
                .await?;
            self.repository.prepare_reconnect_home(&account, &backup)?;
            backend
                .login(
                    LoginRequest {
                        home: home.clone(),
                        expected_email: account.email.clone(),
                        pending_directory: self.repository.pending(),
                        slug: account.slug.clone(),
                        accept_terminal_input,
                        cancel: cancel.clone(),
                    },
                    progress,
                )
                .await?;
            let status = backend.status(&home, false).await?;
            check_cancelled(cancel.as_ref())?;
            check_identity(account.email.as_deref(), status.email.as_deref(), true)?;
            self.apply_status(&mut account, status);
            self.repository.update(|manifest| {
                check_identity_available(
                    manifest,
                    account.provider,
                    account.email.as_deref(),
                    Some(account.id),
                )?;
                let current = matching_account(manifest, &original)?;
                *current = account.clone();
                manifest.account_revisions.insert(account.id, revision);
                Ok(account)
            })
        }
        .await;
        self.finish(&journal, result).await
    }

    pub async fn rename(&self, reference: &str, slug: &str) -> Result<AccountRecord> {
        validate_slug(slug)?;
        let manager = self.clone();
        let reference = reference.to_owned();
        let slug = slug.to_owned();
        tokio::spawn(async move {
            manager.recover_pending().await?;
            let account = manager.resolve(&reference).await?;
            manager.rename_account(account, &slug, true).await
        })
        .await
        .map_err(interrupted)?
    }

    pub(crate) async fn rename_generated(
        &self,
        account: AccountRecord,
        slug: &str,
    ) -> Result<AccountRecord> {
        self.rename_account(account, slug, false).await
    }

    async fn rename_account(
        &self,
        account: AccountRecord,
        slug: &str,
        exact: bool,
    ) -> Result<AccountRecord> {
        let _lock = self.repository.account_lock(account.id).await?;
        self.recover_locked_account(account.id).await?;
        let account = self.unchanged_account(&account)?;
        if account.slug == slug {
            return Ok(account);
        }
        let home = self.check_managed_home(&account)?;
        let _source_lock = self
            .repository
            .slug_lock(account.provider, &account.slug)
            .await?;
        let (slug, _slug_lock) = self.reserve_slug(account.provider, slug, exact).await?;
        let backend = self.backend(account.provider)?;
        let destination = self.repository.home(account.provider, &slug);
        if backend
            .has_credentials(CredentialLocation::Home(&destination))
            .await?
        {
            return Err(Error::SlugTaken(format!("{}/{slug}", account.provider)));
        }
        let journal = Journal {
            account: account.clone(),
            kind: Operation::Rename { slug: slug.clone() },
        };
        journal.save(&self.repository)?;

        let result = async {
            backend
                .stage_credentials(CredentialLocation::Home(&home), &destination)
                .await?;
            move_directory(&home, &destination)?;
            backend.verify_credentials(&destination).await?;
            self.repository.update(|manifest| {
                if manifest
                    .accounts
                    .iter()
                    .any(|entry| entry.provider == account.provider && entry.slug == slug)
                {
                    return Err(Error::SlugTaken(format!("{}/{slug}", account.provider)));
                }
                let current = matching_account(manifest, &account)?;
                current.slug = slug;
                let renamed = current.clone();
                manifest.mark_changed(account.id);
                Ok(renamed)
            })
        }
        .await;
        self.finish(&journal, result).await
    }

    pub async fn remove(&self, reference: &str) -> Result<AccountRecord> {
        let manager = self.clone();
        let reference = reference.to_owned();
        tokio::spawn(async move { manager.remove_inner(&reference).await })
            .await
            .map_err(interrupted)?
    }

    async fn remove_inner(&self, reference: &str) -> Result<AccountRecord> {
        self.recover_pending().await?;
        let original = self.resolve(reference).await?;
        let _lock = self.repository.account_lock(original.id).await?;
        self.recover_locked_account(original.id).await?;
        let account = self.unchanged_account(&original)?;
        // Prevent a new add from claiming this path until credential cleanup
        // has finished, even when the record is already gone.
        let _slug_lock = self
            .repository
            .slug_lock(account.provider, &account.slug)
            .await?;
        self.backend(account.provider)?;
        let home = self.home(&account);
        let journal = Journal {
            account: account.clone(),
            kind: Operation::Remove,
        };
        let backup = journal.backup_home(&self.repository);
        self.ensure_backup_available(&backup, account.provider)
            .await?;
        journal.save(&self.repository)?;

        let result = async {
            if exists(&home)? {
                let home = self.check_managed_home(&account)?;
                move_directory(&home, &backup)?;
            }
            self.repository.update(|manifest| {
                matching_account(manifest, &account)?;
                manifest.accounts.retain(|entry| entry.id != account.id);
                manifest.account_revisions.remove(&account.id);
                manifest.pending_resets.remove(&account.id);
                Ok(account)
            })
        }
        .await;
        self.finish(&journal, result).await
    }

    pub(crate) fn apply_status(&self, account: &mut AccountRecord, status: AccountStatus) {
        if let Some(email) = status.email {
            account.email = Some(email);
        }
        account.plan = status.plan;
        account.quota = Some(status.quota);
        account.last_updated = Some((self.clock)());
    }

    async fn reserve_slug(
        &self,
        provider: Provider,
        requested: &str,
        exact: bool,
    ) -> Result<(String, fs::File)> {
        let base = if requested.is_empty() {
            "account"
        } else {
            requested
        };
        for number in 1.. {
            let slug = if number == 1 {
                base.to_owned()
            } else {
                let suffix = format!("-{number}");
                let prefix = base[..base.len().min(48 - suffix.len())].trim_end_matches('-');
                format!("{prefix}{suffix}")
            };
            if !self.slug_occupied(provider, &slug)? {
                let lock = self.repository.slug_lock(provider, &slug).await?;
                if !self.slug_occupied(provider, &slug)? {
                    return Ok((slug, lock));
                }
            }
            if exact {
                return Err(Error::SlugTaken(format!("{provider}/{slug}")));
            }
        }
        unreachable!("cannot exhaust the slug namespace")
    }

    fn slug_occupied(&self, provider: Provider, slug: &str) -> Result<bool> {
        if exists(&self.repository.home(provider, slug))?
            || self
                .repository
                .snapshot()?
                .unwrap_or_default()
                .accounts
                .iter()
                .any(|entry| entry.provider == provider && entry.slug == slug)
        {
            return Ok(true);
        }
        for id in Journal::pending(&self.repository)? {
            if Journal::read(&self.repository, id)?
                .is_some_and(|journal| journal.reserves_slug(provider, slug))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn ensure_backup_available(&self, path: &Path, provider: Provider) -> Result<()> {
        if exists(path)?
            || self
                .backend(provider)?
                .has_credentials(CredentialLocation::Home(path))
                .await?
        {
            return Err(Error::RecoveryRequired(format!(
                "a previous credential backup remains at {}",
                path.display()
            )));
        }
        Ok(())
    }
}

pub(super) fn check_identity(
    expected: Option<&str>,
    actual: Option<&str>,
    require_reported: bool,
) -> Result<()> {
    if let Some(expected) = expected
        && (require_reported || actual.is_some())
        && actual.is_none_or(|actual| actual.to_lowercase() != expected.to_lowercase())
    {
        return Err(Error::IdentityMismatch {
            expected: expected.into(),
            actual: actual.unwrap_or("an unknown account").into(),
        });
    }
    Ok(())
}

pub(super) fn check_identity_available(
    manifest: &Manifest,
    provider: Provider,
    email: Option<&str>,
    except: Option<Uuid>,
) -> Result<()> {
    if let Some(email) = email
        && let Some(existing) = manifest.accounts.iter().find(|account| {
            account.provider == provider
                && Some(account.id) != except
                && account
                    .email
                    .as_ref()
                    .is_some_and(|other| other.to_lowercase() == email.to_lowercase())
        })
    {
        return Err(Error::DuplicateIdentity {
            email: email.to_owned(),
            reference: existing.reference(),
        });
    }
    Ok(())
}

pub(super) fn matching_account<'a>(
    manifest: &'a mut Manifest,
    original: &AccountRecord,
) -> Result<&'a mut AccountRecord> {
    manifest
        .accounts
        .iter_mut()
        .find(|entry| {
            entry.id == original.id
                && entry.provider == original.provider
                && entry.slug == original.slug
        })
        .ok_or_else(|| Error::StaleState(original.reference()))
}

pub(super) fn interrupted(error: tokio::task::JoinError) -> Error {
    Error::RecoveryRequired(format!("account operation was interrupted: {error}"))
}

fn check_cancelled(cancel: Option<&watch::Receiver<bool>>) -> Result<()> {
    if cancel.is_some_and(|cancel| *cancel.borrow()) {
        return Err(Error::Cancelled);
    }
    Ok(())
}
