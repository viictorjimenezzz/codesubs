use std::{collections::BTreeSet, fs, io, io::Write, os::unix::fs::symlink, path::Path};

use crate::{Provider, Result};

pub(super) fn link(provider: Provider, home: &Path, source: &Path) -> Result<()> {
    let items: &[&str] = match provider {
        Provider::Codex => &["AGENTS.md", "prompts", "rules", "skills", "hooks.json"],
        Provider::Claude => &[
            "settings.json",
            "CLAUDE.md",
            "agents",
            "commands",
            "hooks",
            "output-styles",
            "plugins",
            "skills",
            "keybindings.json",
        ],
    };
    let ledger = home.join(".subscription-manager-links");
    let old_ledger = home.join(".quota-manager-links");
    let existing = read_ledger(&ledger)?;
    let previous = read_ledger(&old_ledger)?;
    let mut handled: BTreeSet<&str> = existing.lines().chain(previous.lines()).collect();

    for item in items {
        if !handled.contains(item) {
            let target = source.join(item);
            if target.try_exists()? {
                match symlink(&target, home.join(item)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                handled.insert(item);
            }
        }
    }

    let content = handled.into_iter().collect::<Vec<_>>().join("\n");
    if content != existing {
        let mut stage = tempfile::NamedTempFile::new_in(home)?;
        stage.write_all(content.as_bytes())?;
        stage.persist(&ledger).map_err(|error| error.error)?;
    }
    if super::homes::exists(&old_ledger)? {
        fs::remove_file(old_ledger)?;
    }
    Ok(())
}

fn read_ledger(path: &Path) -> Result<String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(fs::read_to_string(path)?),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular link ledger",
        )
        .into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}
