use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
};

use crate::{Error, Result};

/// The manager uses the absolute, lexically normalized vault location as
/// part of Claude Code's Keychain identity. It never resolves symlink aliases.
#[derive(Clone, Debug)]
pub struct ManagerConfig {
    pub vault_root: PathBuf,
    pub user_home: PathBuf,
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
        })
    }

    pub fn from_environment() -> Result<Self> {
        let home = absolute(&env::home_dir().ok_or_else(|| {
            Error::InvalidInput("could not locate the current user's home directory".into())
        })?)?;
        let vault_root = match env::var_os("CODESUBS_HOME").filter(|root| !root.is_empty()) {
            Some(path) => {
                let path = PathBuf::from(path);
                match path.strip_prefix("~") {
                    Ok(suffix) => home.join(suffix),
                    Err(_) => path,
                }
            }
            None => configured_root(&home)?.unwrap_or_else(|| home.join(".codesubs")),
        };
        Self::new(vault_root, home)
    }
}

fn configured_root(home: &Path) -> Result<Option<PathBuf>> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct ConfigFile {
        vault_root: PathBuf,
    }

    let path = home.join(".config/codesubs/config.json");
    let content = match fs::read(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if content.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(Error::InvalidInput(format!(
            "{} must contain a JSON object",
            path.display()
        )));
    }
    let config: ConfigFile = serde_json::from_slice(&content)
        .map_err(|error| Error::InvalidInput(format!("invalid {}: {error}", path.display())))?;
    if !config.vault_root.is_absolute() {
        return Err(Error::InvalidInput(format!(
            "vaultRoot in {} must be absolute",
            path.display()
        )));
    }
    Ok(Some(config.vault_root))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_config_requires_an_absolute_path_and_known_fields() {
        let home = tempfile::tempdir().unwrap();
        assert!(configured_root(home.path()).unwrap().is_none());

        let path = home.path().join(".config/codesubs/config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"vaultRoot":"/tmp/a vault"}"#).unwrap();
        assert_eq!(
            configured_root(home.path()).unwrap(),
            Some("/tmp/a vault".into())
        );

        for content in [
            r#"{"vaultRoot":"relative"}"#,
            r#"{"vaultRoot":"/tmp/vault","typo":true}"#,
            r#"["/tmp/vault"]"#,
            "{",
        ] {
            fs::write(&path, content).unwrap();
            assert!(matches!(
                configured_root(home.path()),
                Err(Error::InvalidInput(_))
            ));
        }
    }
}
