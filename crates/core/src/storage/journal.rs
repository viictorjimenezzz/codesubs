use std::{fs, io, os::unix::fs::OpenOptionsExt, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AccountRecord, Error, Provider, Result};

use super::{Repository, homes, manifest::write_json, validate_slug};

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Journal {
    pub account: AccountRecord,
    pub kind: Operation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "camelCase")]
pub(crate) enum Operation {
    Add { from_default: bool },
    Rename { slug: String },
    Remove,
    Reconnect { revision: Uuid },
}

impl Journal {
    pub fn backup_home(&self, repository: &Repository) -> PathBuf {
        repository
            .pending()
            .join(format!("{}.home", self.account.id))
    }

    pub fn save(&self, repository: &Repository) -> Result<()> {
        self.validate()?;
        write_json(&path(repository, self.account.id), self)
    }

    pub fn delete(&self, repository: &Repository) -> Result<()> {
        let path = path(repository, self.account.id);
        fs::remove_file(&path)?;
        homes::sync_parent(&path);
        Ok(())
    }

    pub fn pending(repository: &Repository) -> Result<Vec<Uuid>> {
        let directory = repository.pending();
        if !homes::exists(&directory)? {
            return Ok(Vec::new());
        }
        homes::require_directory(&directory)?;
        let mut pending = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".operation.json"))
                .and_then(|name| Uuid::parse_str(name).ok())
            {
                pending.push(id);
            }
        }
        Ok(pending)
    }

    pub fn read(repository: &Repository, id: Uuid) -> Result<Option<Self>> {
        let file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path(repository, id))
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !file.metadata()?.is_file() {
            return Err(Error::RecoveryRequired(
                "account recovery record must be a regular file".into(),
            ));
        }
        let journal: Self = serde_json::from_reader(file).map_err(|error| {
            Error::RecoveryRequired(format!("invalid recovery record for account {id}: {error}"))
        })?;
        journal.validate()?;
        if journal.account.id != id {
            return Err(Error::RecoveryRequired(
                "account recovery record has a mismatched ID".into(),
            ));
        }
        Ok(Some(journal))
    }

    pub fn reserves_slug(&self, provider: Provider, slug: &str) -> bool {
        self.account.provider == provider
            && (self.account.slug == slug
                || matches!(&self.kind, Operation::Rename { slug: destination } if destination == slug))
    }

    fn validate(&self) -> Result<()> {
        let slug = &self.account.slug;
        if slug.is_empty()
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(Error::RecoveryRequired(
                "invalid account path in recovery record".into(),
            ));
        }
        if let Operation::Rename { slug } = &self.kind {
            validate_slug(slug)?;
        }
        Ok(())
    }
}

fn path(repository: &Repository, id: Uuid) -> PathBuf {
    repository.pending().join(format!("{id}.operation.json"))
}
