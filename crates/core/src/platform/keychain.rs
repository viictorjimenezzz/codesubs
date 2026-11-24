use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

pub fn service_name(config_directory: &str) -> String {
    let normalized: String = config_directory.nfc().collect();
    let digest = Sha256::digest(normalized.as_bytes());
    format!(
        "Claude Code-credentials-{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

#[cfg(target_os = "macos")]
pub(crate) use macos::{
    has_credentials, read, read_secret, remove, remove_secret, stage, store_secret,
};

/// The Keychain service holding an API-key account's secret.
pub fn gateway_service(reference: &str) -> String {
    format!("codesubs-api-{reference}")
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn store_secret(_: &str, _: &str) -> crate::Result<()> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn read_secret(_: &str) -> crate::Result<Option<String>> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn remove_secret(_: &str) -> crate::Result<()> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
fn unsupported() -> crate::Error {
    crate::Error::Unsupported("API keys are stored in the macOS Keychain.".into())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::env;
    use std::fmt::Write;
    use std::path::Path;
    use std::time::Duration;

    use crate::providers::CredentialLocation;
    use crate::runtime::{ProcessOutput, run};
    use crate::{Error, Result};

    use super::service_name;

    const NOT_FOUND: i32 = 44; // Low eight bits of errSecItemNotFound.
    const SECURITY: &str = "/usr/bin/security";

    pub async fn read(location: CredentialLocation<'_>) -> Result<Option<Vec<u8>>> {
        read_service(&service(location)?).await
    }

    pub async fn stage(source: CredentialLocation<'_>, destination: &Path) -> Result<()> {
        let source_service = service(source)?;
        let destination = service(CredentialLocation::Home(destination))?;
        if service_exists(&destination).await? {
            return Err(Error::Provider(
                "The destination Claude Keychain entry already exists.".into(),
            ));
        }
        let Some(secret) = read_service(&source_service).await? else {
            return match source {
                CredentialLocation::Default => Err(Error::Provider(
                    "The default Claude login has no Keychain entry to adopt.".into(),
                )),
                CredentialLocation::Home(_) => Ok(()),
            };
        };

        let account = account_name();
        // The secret goes only over a pipe. Omitting -U makes adding atomic
        // when another process creates an entry after the existence check.
        let mut command =
            format!("add-generic-password -a \"{account}\" -s \"{destination}\" -X \"");
        for byte in &secret {
            write!(&mut command, "{byte:02x}").expect("writing to a string");
        }
        command.push_str("\"\n");
        if command.len() >= 4096 {
            return Err(Error::Provider(
                "The Claude credential exceeds the macOS Keychain tool's safe input length.".into(),
            ));
        }
        let result = security(["-i"], Some(command.as_bytes())).await?;
        if !result.status.success() {
            return Err(status_error("stage", &result));
        }
        if read_service(&destination).await?.as_deref() != Some(secret.as_slice()) {
            return Err(Error::Provider(
                "The staged Claude credential could not be verified.".into(),
            ));
        }
        Ok(())
    }

    pub async fn has_credentials(location: CredentialLocation<'_>) -> Result<bool> {
        service_exists(&service(location)?).await
    }

    pub async fn remove(location: CredentialLocation<'_>) -> Result<()> {
        remove_secret(&service(location)?).await
    }

    /// Plain API keys are stored with a marker so a key made only of hex
    /// digits is never mistaken for the hex form `security` prints for
    /// non-ASCII entries.
    const SECRET_MARKER: &str = "codesubs-key:";

    pub async fn store_secret(service: &str, secret: &str) -> Result<()> {
        if secret.is_empty() || !secret.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(Error::InvalidInput(
                "an API key must be printable ASCII without spaces".into(),
            ));
        }
        let mut command = format!(
            "add-generic-password -U -a \"{}\" -s \"{service}\" -X \"",
            account_name()
        );
        for byte in format!("{SECRET_MARKER}{secret}").bytes() {
            write!(&mut command, "{byte:02x}").expect("writing to a string");
        }
        command.push_str("\"\n");
        let result = security(["-i"], Some(command.as_bytes())).await?;
        if !result.status.success() {
            return Err(status_error("store", &result));
        }
        Ok(())
    }

    pub async fn read_secret(service: &str) -> Result<Option<String>> {
        Ok(read_service(service)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| text.strip_prefix(SECRET_MARKER).map(str::to_owned)))
    }

    pub async fn remove_secret(service: &str) -> Result<()> {
        let account = account_name();
        let result = security(
            ["delete-generic-password", "-a", &account, "-s", service],
            None,
        )
        .await?;
        if result.status.success() || result.status.code() == Some(NOT_FOUND) {
            return Ok(());
        }
        Err(status_error("remove", &result))
    }

    async fn service_exists(service: &str) -> Result<bool> {
        let account = account_name();
        let result = security(
            ["find-generic-password", "-a", &account, "-s", service],
            None,
        )
        .await?;
        if result.status.success() {
            Ok(true)
        } else if result.status.code() == Some(NOT_FOUND) {
            Ok(false)
        } else {
            Err(status_error("find", &result))
        }
    }

    async fn read_service(service: &str) -> Result<Option<Vec<u8>>> {
        let account = account_name();
        let mut result = security(
            ["find-generic-password", "-a", &account, "-s", service, "-w"],
            None,
        )
        .await?;
        if result.status.code() == Some(NOT_FOUND) {
            return Ok(None);
        }
        if !result.status.success() {
            return Err(status_error("read", &result));
        }
        if result.stdout.last() == Some(&b'\n') {
            result.stdout.pop();
        }
        // security renders non-ASCII entries as hex. A plain Claude
        // credential is a JSON object and cannot be composed solely of hex.
        let secret = decode_hex(&result.stdout).unwrap_or(result.stdout);
        Ok(Some(secret))
    }

    async fn security<const N: usize>(
        args: [&str; N],
        input: Option<&[u8]>,
    ) -> Result<ProcessOutput> {
        run(
            Path::new(SECURITY),
            args,
            None,
            input,
            Duration::from_secs(20),
            "macOS Keychain operation",
        )
        .await
    }

    fn status_error(action: &str, result: &ProcessOutput) -> Error {
        Error::Provider(format!(
            "Could not {action} the Keychain entry ({0}).",
            result.status
        ))
    }

    fn service(location: CredentialLocation<'_>) -> Result<String> {
        match location {
            CredentialLocation::Default => Ok("Claude Code-credentials".to_owned()),
            CredentialLocation::Home(home) => {
                let config_directory = home.to_str().ok_or_else(|| {
                    Error::Provider("The Claude configuration directory is not valid UTF-8.".into())
                })?;
                Ok(service_name(config_directory))
            }
        }
    }

    fn account_name() -> String {
        let user = env::var("USER").ok().or_else(|| {
            let id = std::process::Command::new("/usr/bin/id")
                .arg("-un")
                .output()
                .ok()?;
            id.status
                .success()
                .then(|| String::from_utf8_lossy(&id.stdout).trim().to_owned())
        });
        user.filter(|user| {
            !user.is_empty()
                && user
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
        .unwrap_or_else(|| "claude-code-user".to_owned())
    }

    fn decode_hex(bytes: &[u8]) -> Option<Vec<u8>> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
            return None;
        }
        bytes
            .chunks_exact(2)
            .map(|pair| {
                let hi = (pair[0] as char).to_digit(16)?;
                let lo = (pair[1] as char).to_digit(16)?;
                Some((hi * 16 + lo) as u8)
            })
            .collect()
    }
}
