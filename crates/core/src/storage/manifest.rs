use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::Repository;
use crate::{AccountRecord, Error, Provider, Result};

const CURRENT_VERSION: u32 = 3;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub root: Option<String>,
    pub accounts: Vec<AccountRecord>,
    #[serde(default)]
    pub account_revisions: BTreeMap<Uuid, Uuid>,
    #[serde(default)]
    pub pending_resets: BTreeMap<Uuid, Uuid>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            revision: 0,
            root: None,
            accounts: Vec::new(),
            account_revisions: BTreeMap::new(),
            pending_resets: BTreeMap::new(),
        }
    }
}

impl Manifest {
    pub fn read(file: File, repository: &Repository) -> Result<Self> {
        let raw: Value = serde_json::from_reader(file).map_err(corrupt)?;
        let version = raw
            .get("version")
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
            .ok_or_else(|| Error::CorruptManifest("missing or invalid version".into()))?;
        if version != CURRENT_VERSION {
            return Err(Error::UnsupportedManifest(version));
        }
        let manifest = Self::deserialize(&raw).map_err(corrupt)?;
        manifest.validate()?;
        manifest.check_root(repository)?;
        if let Some(entries) = raw.get("accounts").and_then(Value::as_array) {
            for (entry, account) in entries.iter().zip(&manifest.accounts) {
                if account.provider != Provider::Claude {
                    continue;
                }
                let home = repository.home(account.provider, &account.slug);
                let path = home.to_str().expect("validated vault path");
                let environment = account.provider.home_environment_variable();
                for hint in [
                    entry.get("home"),
                    entry.get("env").and_then(|env| env.get(environment)),
                ]
                .into_iter()
                .flatten()
                {
                    if !hint.is_null() && hint.as_str() != Some(path) {
                        return Err(Error::RecoveryRequired(format!(
                            "stored path for {} differs from {}. Restore its exact Claude configuration path before continuing",
                            account.reference(),
                            home.display()
                        )));
                    }
                }
            }
        }
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for account in &self.accounts {
            let valid_path = !account.slug.is_empty()
                && account.slug.len() <= 255
                && account
                    .slug
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && account
                    .slug
                    .bytes()
                    .any(|byte| byte.is_ascii_alphanumeric());
            if !valid_path
                || !ids.insert(account.id)
                || !paths.insert((account.provider, &account.slug))
            {
                return Err(Error::CorruptManifest(
                    "invalid or duplicate account path or ID".into(),
                ));
            }
        }
        if self
            .account_revisions
            .keys()
            .chain(self.pending_resets.keys())
            .any(|id| !ids.contains(id))
        {
            return Err(Error::CorruptManifest(
                "account metadata references a missing account".into(),
            ));
        }
        Ok(())
    }

    fn check_root(&self, repository: &Repository) -> Result<()> {
        if let Some(root) = &self.root
            && root != repository.root().to_str().expect("validated vault path")
            && self
                .accounts
                .iter()
                .any(|account| account.provider == Provider::Claude)
        {
            return Err(Error::RecoveryRequired(format!(
                "vault records Claude credentials under {root}. Restore that exact vault path before continuing"
            )));
        }
        Ok(())
    }

    pub fn write(&mut self, repository: &Repository) -> Result<()> {
        self.validate()?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| Error::CorruptManifest("manifest revision is exhausted".into()))?;
        self.revision = revision;
        self.root = Some(
            repository
                .root()
                .to_str()
                .expect("validated vault path")
                .to_owned(),
        );
        self.version = CURRENT_VERSION;

        let mut raw = serde_json::to_value(&*self).map_err(corrupt)?;
        if let Some(entries) = raw.get_mut("accounts").and_then(Value::as_array_mut) {
            for (entry, account) in entries.iter_mut().zip(&self.accounts) {
                let home = repository.home(account.provider, &account.slug);
                let path = home.to_str().expect("validated account path");
                let object = entry
                    .as_object_mut()
                    .expect("account serializes as an object");
                object.insert("reference".into(), account.reference().into());
                object.insert("home".into(), path.into());
                object.insert(
                    "env".into(),
                    serde_json::json!({account.provider.home_environment_variable(): path}),
                );
            }
        }
        write_json(&repository.manifest_path(), &raw)
    }

    pub fn mark_changed(&mut self, id: Uuid) {
        self.account_revisions.insert(id, Uuid::new_v4());
    }
}

pub(super) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let directory = path.parent().expect("vault file has a parent");
    let mut stage = tempfile::NamedTempFile::new_in(directory)?;
    stage
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer_pretty(stage.as_file_mut(), value).map_err(std::io::Error::other)?;
    stage.write_all(b"\n")?;
    stage.as_file().sync_all()?;
    stage.persist(path).map_err(|error| error.error)?;
    super::homes::sync_parent(path);
    Ok(())
}

fn corrupt(error: serde_json::Error) -> Error {
    Error::CorruptManifest(error.to_string())
}
