use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    Manifest, Repository,
    homes::{exists, move_directory},
    manifest::write_json,
    sanitize_slug, slug_from_email, validate_slug,
};
use crate::{AccountRecord, Error, Provider, Result};

#[derive(Deserialize)]
struct LegacyAccount {
    id: Uuid,
    name: Option<String>,
    email: Option<String>,
    plan: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct PlannedMove {
    account: AccountRecord,
    uppercase_profile_id: bool,
}

impl PlannedMove {
    fn source(&self, legacy_root: &Path) -> PathBuf {
        let id = self.account.id.to_string();
        let folder = if self.uppercase_profile_id {
            id.to_uppercase()
        } else {
            id
        };
        legacy_root.join("Profiles").join(folder)
    }

    fn target(&self, repository: &Repository) -> PathBuf {
        repository.home(Provider::Codex, &self.account.slug)
    }
}

pub(super) fn journal_path(repository: &Repository) -> PathBuf {
    repository.pending().join("codex-quota-migration.json")
}

/// Caller holds the manifest lock. The plan is durable before moving any
/// profiles, and each planned path identifies a completed move on restart.
pub(super) fn migrate_codex(repository: &Repository, legacy: &Path) -> Result<()> {
    let journal = journal_path(repository);
    let plan: Vec<PlannedMove> = if exists(&journal)? {
        let file = fs::File::open(&journal)?;
        serde_json::from_reader(file).map_err(|error| {
            Error::RecoveryRequired(format!("invalid Codex Quota migration journal: {error}"))
        })?
    } else {
        let plan = plan_migration(repository, legacy)?;
        write_json(&journal, &plan)?;
        plan
    };

    let mut ids = HashSet::new();
    let mut slugs = HashSet::new();
    for entry in &plan {
        validate_slug(&entry.account.slug)?;
        if entry.account.provider != Provider::Codex
            || !ids.insert(entry.account.id)
            || !slugs.insert(&entry.account.slug)
        {
            return Err(Error::RecoveryRequired(
                "invalid Codex Quota migration plan".into(),
            ));
        }
    }

    let result = (|| {
        for entry in &plan {
            let source = entry.source(legacy);
            let target = entry.target(repository);
            match (exists(&source)?, exists(&target)?) {
                (true, false) => move_directory(&source, &target)?,
                (false, true) => super::homes::require_directory(&target)?,
                _ => {
                    return Err(Error::RecoveryRequired(format!(
                        "cannot identify the sole legacy profile for {}",
                        entry.account.reference()
                    )));
                }
            }
        }
        let mut manifest = Manifest {
            accounts: plan.iter().map(|entry| entry.account.clone()).collect(),
            ..Manifest::default()
        };
        manifest.write(repository)
    })();

    if let Err(error) = result {
        for entry in plan.iter().rev() {
            let source = entry.source(legacy);
            let target = entry.target(repository);
            if !exists(&source)? && exists(&target)? {
                move_directory(&target, &source).map_err(|rollback| {
                    Error::RecoveryRequired(format!(
                        "legacy migration failed ({error}), and {} could not be restored to {} ({rollback})",
                        target.display(), source.display()
                    ))
                })?;
            }
        }
        // Keep the plan if any move's ownership was ambiguous. A subsequent
        // attempt must use exactly the same paths after the conflict is fixed.
        return Err(error);
    }

    fs::remove_file(journal).ok();
    Ok(())
}

fn plan_migration(repository: &Repository, legacy: &Path) -> Result<Vec<PlannedMove>> {
    let source_manifest = legacy.join("accounts.json");
    let records: Vec<LegacyAccount> = serde_json::from_reader(fs::File::open(source_manifest)?)
        .map_err(|error| Error::CorruptManifest(format!("legacy Codex Quota manifest: {error}")))?;
    let mut taken = HashSet::new();
    let mut plan = Vec::new();

    for record in records {
        let profiles = legacy.join("Profiles");
        let uppercase_profile_id = exists(&profiles.join(record.id.to_string().to_uppercase()))?;
        if !uppercase_profile_id && !exists(&profiles.join(record.id.to_string()))? {
            continue;
        }
        let base = record
            .email
            .as_deref()
            .map(slug_from_email)
            .unwrap_or_else(|| sanitize_slug(record.name.as_deref().unwrap_or_default()));
        let base = if base.is_empty() { "account" } else { &base };
        let mut slug = base.to_owned();
        let mut number = 2_u32;
        while taken.contains(&slug) || exists(&repository.home(Provider::Codex, &slug))? {
            let suffix = format!("-{number}");
            slug = format!(
                "{}{suffix}",
                base[..base.len().min(48 - suffix.len())].trim_end_matches('-')
            );
            number += 1;
        }
        taken.insert(slug.clone());
        plan.push(PlannedMove {
            account: AccountRecord {
                id: record.id,
                provider: Provider::Codex,
                slug,
                email: record.email,
                plan: record.plan,
                quota: None,
                last_updated: None,
            },
            uppercase_profile_id,
        });
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ManagerConfig;

    #[test]
    fn migration_restarts_from_a_persisted_plan_with_a_finished_move() {
        let scratch = tempfile::TempDir::new().unwrap();
        let user = scratch.path().join("user");
        let legacy = user.join("Library/Application Support/CodexQuota");
        let mut config = ManagerConfig::new(scratch.path().join("vault"), user).unwrap();
        config.migrate_legacy = true;
        let repository = Repository::new(config);
        repository.initialize().unwrap();

        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let legacy_records = serde_json::json!([
            { "id": first, "email": "one@example.test" },
            { "id": second, "email": "two@example.test" }
        ]);
        fs::create_dir_all(legacy.join("Profiles")).unwrap();
        fs::write(legacy.join("accounts.json"), legacy_records.to_string()).unwrap();
        for id in [first, second] {
            let profile = legacy.join("Profiles").join(id.to_string().to_uppercase());
            fs::create_dir(&profile).unwrap();
            fs::write(profile.join("auth.json"), id.to_string()).unwrap();
        }

        let plan = plan_migration(&repository, &legacy).unwrap();
        write_json(&journal_path(&repository), &plan).unwrap();
        move_directory(&plan[0].source(&legacy), &plan[0].target(&repository)).unwrap();

        repository.migrate_legacy().unwrap();
        let manifest = repository.snapshot().unwrap().unwrap();
        assert_eq!(manifest.accounts.len(), 2);
        for account in manifest.accounts {
            assert!(account.id == first || account.id == second);
            let home = repository.home(account.provider, &account.slug);
            assert_eq!(
                fs::read_to_string(home.join("auth.json")).unwrap(),
                account.id.to_string()
            );
        }
        assert!(!journal_path(&repository).exists());
    }
}
