use std::{
    env,
    path::{Component, Path, PathBuf},
};

use crate::{Error, Result};

/// The manager uses the absolute, lexically normalized vault location as
/// part of Claude Code's Keychain identity. It never resolves symlink aliases.
#[derive(Clone, Debug)]
pub struct ManagerConfig {
    pub vault_root: PathBuf,
    pub user_home: PathBuf,
    pub(crate) migrate_legacy: bool,
}

impl ManagerConfig {
    pub fn new(vault_root: PathBuf, user_home: PathBuf) -> Result<Self> {
        let user_home = absolute(&user_home)?;
        let vault_root = absolute(&vault_root)?;
        if vault_root == user_home || vault_root.parent().is_none() {
            return Err(Error::InvalidInput(
                "choose a dedicated vault directory".into(),
            ));
        }
        if vault_root.to_str().is_none() || user_home.to_str().is_none() {
            return Err(Error::InvalidInput(
                "vault and user home paths must use UTF-8".into(),
            ));
        }
        Ok(Self {
            vault_root,
            user_home,
            migrate_legacy: false,
        })
    }

    pub fn from_environment() -> Result<Self> {
        let home = absolute(&env::home_dir().ok_or_else(|| {
            Error::InvalidInput("could not locate the current user's home directory".into())
        })?)?;
        let override_root =
            env::var_os("SUBSCRIPTION_MANAGER_HOME").filter(|root| !root.is_empty());
        let vault_root = override_root.as_ref().map(PathBuf::from).map_or_else(
            || {
                let current = home.join(".subscription-manager");
                let previous = home.join(".quota-manager");
                if !current.join("accounts.json").exists()
                    && previous.join("accounts.json").exists()
                {
                    previous
                } else {
                    current
                }
            },
            |path| {
                path.strip_prefix("~")
                    .map_or_else(|_| path.clone(), |suffix| home.join(suffix))
            },
        );
        let mut config = Self::new(vault_root, home)?;
        config.migrate_legacy = override_root.is_none();
        Ok(config)
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let input = if path.is_absolute() {
        path.to_owned()
    } else {
        env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in input.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component),
        }
    }
    Ok(normalized)
}
