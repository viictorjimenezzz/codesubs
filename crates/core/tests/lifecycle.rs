use std::{
    collections::HashMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use codesubs_core::*;
use tempfile::TempDir;
use tokio::sync::{Notify, watch};
use uuid::Uuid;

struct FakeProvider {
    provider: Provider,
    login_identity: Mutex<Option<String>>,
    credentials: Mutex<HashMap<PathBuf, String>>,
    verify_fails: AtomicBool,
    block_verify: AtomicBool,
    verify_entered: Notify,
    verify_release: Notify,
    login_entered: Notify,
    login_barrier: Option<(PathBuf, usize)>,
    reset_attempts: Mutex<Vec<Uuid>>,
    reset_failures: AtomicUsize,
    reset_applied: AtomicBool,
}

impl FakeProvider {
    fn new(provider: Provider) -> Self {
        Self {
            provider,
            login_identity: Mutex::new(None),
            credentials: Mutex::new(HashMap::new()),
            verify_fails: AtomicBool::new(false),
            block_verify: AtomicBool::new(false),
            verify_entered: Notify::new(),
            verify_release: Notify::new(),
            login_entered: Notify::new(),
            login_barrier: None,
            reset_attempts: Mutex::new(Vec::new()),
            reset_failures: AtomicUsize::new(0),
            reset_applied: AtomicBool::new(false),
        }
    }

    fn stored_identity(&self, home: &Path) -> Result<String> {
        match self.provider {
            Provider::Codex => Ok(fs::read_to_string(home.join("auth.json"))?),
            Provider::Claude | Provider::Antigravity => self
                .credentials
                .lock()
                .unwrap()
                .get(home)
                .cloned()
                .ok_or(Error::Provider("fake credentials missing".into())),
        }
    }

    fn location(location: CredentialLocation<'_>) -> &Path {
        match location {
            CredentialLocation::Default => Path::new("<default>"),
            CredentialLocation::Home(home) => home,
        }
    }
}

#[async_trait]
impl ProviderBackend for FakeProvider {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            read_reset_balance: true,
            redeem_reset: self.provider == Provider::Codex,
        }
    }

    fn executable(&self) -> Option<PathBuf> {
        None
    }

    async fn status(&self, home: &Path, _refreshing_token: bool) -> Result<AccountStatus> {
        let email = self.stored_identity(home)?;
        let applied = self.reset_applied.load(Ordering::SeqCst);
        Ok(AccountStatus {
            email: Some(email),
            plan: Some("fake".into()),
            quota: QuotaSnapshot {
                windows: vec![QuotaWindow {
                    kind: QuotaWindowKind::Weekly,
                    label: "Weekly".into(),
                    used_percent: if applied { 0 } else { 75 },
                    resets_at: None,
                    window_duration_minutes: Some(10080),
                    detail: None,
                }],
                reset_credits: Some(ResetCredits {
                    available: if applied { 0 } else { 1 },
                    next_expiry: None,
                    grants: None,
                }),
                credits: None,
            },
        })
    }

    async fn login(&self, mut request: LoginRequest, _progress: LoginProgress) -> Result<()> {
        let email = self
            .login_identity
            .lock()
            .unwrap()
            .clone()
            .or(request.expected_email)
            .unwrap_or("fallback@example.test".into());
        fs::create_dir_all(request.home.join("sessions"))?;
        fs::write(request.home.join("sessions/fresh"), "new login state")?;
        fs::write(request.home.join("config.toml"), "provider defaults")?;
        match self.provider {
            Provider::Codex => fs::write(request.home.join("auth.json"), email)?,
            Provider::Claude | Provider::Antigravity => {
                self.credentials.lock().unwrap().insert(request.home, email);
            }
        }
        self.login_entered.notify_one();

        if let Some((directory, count)) = &self.login_barrier {
            fs::write(directory.join(request.slug), "")?;
            tokio::time::timeout(Duration::from_secs(5), async {
                while fs::read_dir(directory).unwrap().count() < *count {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .map_err(|_| Error::Timeout("test barrier timed out".into()))?;
        }

        if let Some(cancel) = &mut request.cancel {
            cancel.changed().await.map_err(|_| Error::Cancelled)?;
            if *cancel.borrow() {
                return Err(Error::Cancelled);
            }
        }
        Ok(())
    }

    async fn verify_credentials(&self, home: &Path) -> Result<()> {
        if self.block_verify.load(Ordering::SeqCst) {
            self.verify_entered.notify_one();
            self.verify_release.notified().await;
        }
        if self.verify_fails.load(Ordering::SeqCst) {
            return Err(Error::Provider(
                "fake credential verification failed".into(),
            ));
        }
        self.stored_identity(home)?;
        Ok(())
    }

    async fn stage_credentials(
        &self,
        source: CredentialLocation<'_>,
        destination: &Path,
    ) -> Result<()> {
        if self.provider == Provider::Claude {
            let mut credentials = self.credentials.lock().unwrap();
            if credentials.contains_key(destination) {
                return Err(Error::Provider(
                    "fake destination already holds credentials".into(),
                ));
            }
            if let Some(credential) = credentials.get(Self::location(source)).cloned() {
                credentials.insert(destination.to_owned(), credential);
            }
        }
        Ok(())
    }

    async fn remove_credentials(&self, location: CredentialLocation<'_>) -> Result<()> {
        if self.provider == Provider::Claude {
            self.credentials
                .lock()
                .unwrap()
                .remove(Self::location(location));
        }
        Ok(())
    }

    async fn has_credentials(&self, location: CredentialLocation<'_>) -> Result<bool> {
        Ok(self
            .credentials
            .lock()
            .unwrap()
            .contains_key(Self::location(location)))
    }

    async fn redeem_reset(&self, _home: &Path, key: Uuid) -> Result<ResetOutcome> {
        self.reset_attempts.lock().unwrap().push(key);
        self.reset_applied.store(true, Ordering::SeqCst);
        if self
            .reset_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            return Err(Error::Provider("fake transport lost the response".into()));
        }
        Ok(ResetOutcome::AlreadyRedeemed)
    }
}

fn fake_manager(root: &Path, provider: Arc<FakeProvider>) -> Codesubs {
    let config = ManagerConfig::new(root.join("vault"), root.join("user")).unwrap();
    Codesubs::with_backends(config, [provider as Arc<dyn ProviderBackend>]).unwrap()
}

fn progress() -> LoginProgress {
    Arc::new(|_| ())
}

fn options(slug: &str, email: &str) -> AddOptions {
    AddOptions {
        slug: Some(slug.into()),
        expected_email: Some(email.into()),
        ..AddOptions::default()
    }
}

#[tokio::test]
async fn wrong_default_identity_restores_codex_login() {
    let scratch = TempDir::new().unwrap();
    let manager = fake_manager(scratch.path(), Arc::new(FakeProvider::new(Provider::Codex)));
    let default = manager.config().user_home.join(".codex");
    fs::create_dir_all(&default).unwrap();
    fs::write(default.join("auth.json"), "actual@example.test").unwrap();

    let result = manager
        .add(
            Provider::Codex,
            AddOptions {
                from_default: true,
                ..options("work", "expected@example.test")
            },
            progress(),
        )
        .await;
    assert!(matches!(result, Err(Error::IdentityMismatch { .. })));
    assert!(manager.list().await.unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(default.join("auth.json")).unwrap(),
        "actual@example.test"
    );
    assert!(!manager.config().vault_root.join("codex/work").exists());
}

#[tokio::test]
async fn rename_verification_rolls_back_folder_and_path_keyed_credentials() {
    let scratch = TempDir::new().unwrap();
    let backend = Arc::new(FakeProvider::new(Provider::Claude));
    let manager = fake_manager(scratch.path(), backend.clone());
    let record = manager
        .add(
            Provider::Claude,
            options("work", "owner@example.test"),
            progress(),
        )
        .await
        .unwrap();
    let original = manager.home(&record);
    let destination = original.with_file_name("renamed");
    fs::write(original.join("history"), "original file").unwrap();

    backend.verify_fails.store(true, Ordering::SeqCst);
    assert!(matches!(
        manager.rename("claude/work", "renamed").await,
        Err(Error::Provider(_))
    ));
    assert_eq!(
        fs::read_to_string(original.join("history")).unwrap(),
        "original file"
    );
    assert_eq!(
        backend.stored_identity(&original).unwrap(),
        "owner@example.test"
    );
    assert!(backend.stored_identity(&destination).is_err());
    assert!(!destination.exists());
    assert_eq!(manager.resolve("work").await.unwrap().id, record.id);

    backend.verify_fails.store(false, Ordering::SeqCst);
    let renamed = manager.rename("claude/work", "renamed").await.unwrap();
    assert_eq!(renamed.id, record.id);
    assert_eq!(
        backend.stored_identity(&destination).unwrap(),
        "owner@example.test"
    );
    assert!(!original.exists());
    assert!(backend.stored_identity(&original).is_err());
    let execution = manager.env("claude/renamed").await.unwrap();
    assert_eq!(
        execution.set["CLAUDE_CONFIG_DIR"],
        destination.to_str().unwrap()
    );
    assert!(execution.unset.contains(&"ANTHROPIC_API_KEY".into()));
}

#[tokio::test]
async fn reconnect_rejects_a_different_identity_and_restores_the_original() {
    let scratch = TempDir::new().unwrap();
    let backend = Arc::new(FakeProvider::new(Provider::Claude));
    let manager = fake_manager(scratch.path(), backend.clone());
    let record = manager
        .add(
            Provider::Claude,
            options("work", "owner@example.test"),
            progress(),
        )
        .await
        .unwrap();
    let original = manager.home(&record);
    fs::write(original.join("history"), "original file").unwrap();
    *backend.login_identity.lock().unwrap() = Some("other@example.test".into());

    let result = manager.reconnect("claude/work", progress(), false).await;
    assert!(matches!(result, Err(Error::IdentityMismatch { .. })));
    assert_eq!(
        backend.stored_identity(&original).unwrap(),
        "owner@example.test"
    );
    assert_eq!(
        fs::read_to_string(original.join("history")).unwrap(),
        "original file"
    );
    assert_eq!(backend.credentials.lock().unwrap().len(), 1);
    assert_eq!(manager.resolve("work").await.unwrap().id, record.id);
}

#[tokio::test]
async fn cancelled_login_removes_staged_credentials_and_home() {
    let scratch = TempDir::new().unwrap();
    let backend = Arc::new(FakeProvider::new(Provider::Claude));
    let manager = fake_manager(scratch.path(), backend.clone());
    let (cancel, receiver) = watch::channel(false);
    let operation = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .add(
                    Provider::Claude,
                    AddOptions {
                        cancel: Some(receiver),
                        ..options("work", "owner@example.test")
                    },
                    progress(),
                )
                .await
        }
    });
    backend.login_entered.notified().await;
    cancel.send(true).unwrap();
    assert!(matches!(operation.await.unwrap(), Err(Error::Cancelled)));
    assert!(manager.list().await.unwrap().is_empty());
    assert!(backend.credentials.lock().unwrap().is_empty());
    assert!(!manager.config().vault_root.join("claude/work").exists());
}

#[tokio::test]
async fn refresh_does_not_publish_a_snapshot_taken_before_rename() {
    let scratch = TempDir::new().unwrap();
    let backend = Arc::new(FakeProvider::new(Provider::Codex));
    let manager = fake_manager(scratch.path(), backend.clone());
    manager
        .add(
            Provider::Codex,
            options("work", "owner@example.test"),
            progress(),
        )
        .await
        .unwrap();

    backend.block_verify.store(true, Ordering::SeqCst);
    let rename = tokio::spawn({
        let manager = manager.clone();
        async move { manager.rename("codex/work", "renamed").await }
    });
    backend.verify_entered.notified().await;

    let mut refresh = Box::pin(manager.refresh(Some("codex/work")));
    tokio::select! {
        biased;
        result = &mut refresh => panic!("refresh ran past rename guard: {result:?}"),
        _ = std::future::ready(()) => {}
    }
    backend.verify_release.notify_one();
    rename.await.unwrap().unwrap();
    let report = refresh.await.unwrap();
    assert!(report.accounts.is_empty());
    assert_eq!(report.failures[0].code, "stale_state");
    assert_eq!(
        manager.list().await.unwrap()[0].reference(),
        "codex/renamed"
    );
}

#[tokio::test]
async fn reset_keeps_the_same_key_across_uncertain_replies_and_later_invocations() {
    let scratch = TempDir::new().unwrap();
    let backend = Arc::new(FakeProvider::new(Provider::Codex));
    let manager = fake_manager(scratch.path(), backend.clone());
    manager
        .add(
            Provider::Codex,
            options("work", "owner@example.test"),
            progress(),
        )
        .await
        .unwrap();

    backend.reset_failures.store(2, Ordering::SeqCst);
    let Err(Error::ResetUncertain { key, .. }) = manager.redeem_reset("codex/work", None).await
    else {
        panic!("expected uncertain outcome");
    };
    let restarted = fake_manager(scratch.path(), backend.clone());
    assert_eq!(
        restarted
            .preview_reset("codex/work")
            .await
            .unwrap()
            .available,
        Some(0)
    );
    let receipt = restarted.redeem_reset("codex/work", None).await.unwrap();
    assert_eq!(receipt.idempotency_key, key);
    assert_eq!(receipt.outcome, ResetOutcome::AlreadyRedeemed);
    assert_eq!(*backend.reset_attempts.lock().unwrap(), vec![key; 3]);
}

#[test]
fn separate_processes_preserve_concurrent_adds_and_enforce_unique_identity() {
    let scratch = TempDir::new().unwrap();
    let barrier = scratch.path().join("barrier");
    fs::create_dir(&barrier).unwrap();
    let current = std::env::current_exe().unwrap();
    let mut children: Vec<_> = [
        ("one", "first@example.test"),
        ("two", "second@example.test"),
        ("duplicate", "FIRST@example.test"),
    ]
    .into_iter()
    .map(|(slug, email)| {
        Command::new(&current)
            .args(["--exact", "child_writer"])
            .env("SM_TEST_WRITER_ROOT", scratch.path())
            .env("SM_TEST_WRITER_SLUG", slug)
            .env("SM_TEST_WRITER_EMAIL", email)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    })
    .collect();

    for child in children.drain(..) {
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
    let manager = fake_manager(scratch.path(), Arc::new(FakeProvider::new(Provider::Codex)));
    let accounts = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(manager.list())
        .unwrap();
    assert_eq!(accounts.len(), 2);
    assert!(
        accounts
            .iter()
            .any(|account| account.reference() == "codex/two")
    );
    let manifest = manager.config().vault_root.join("accounts.json");
    assert_eq!(
        fs::metadata(manifest).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&manager.config().vault_root)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[tokio::test]
async fn child_writer() {
    let Some(root) = std::env::var_os("SM_TEST_WRITER_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let slug = std::env::var("SM_TEST_WRITER_SLUG").unwrap();
    let email = std::env::var("SM_TEST_WRITER_EMAIL").unwrap();
    let mut backend = FakeProvider::new(Provider::Codex);
    backend.login_barrier = Some((root.join("barrier"), 3));
    let manager = fake_manager(&root, Arc::new(backend));
    let result = manager
        .add(Provider::Codex, options(&slug, &email), progress())
        .await;
    assert!(
        result.is_ok() || matches!(result, Err(Error::DuplicateIdentity { .. })),
        "{result:?}"
    );
}

#[test]
fn selection_prefers_fresh_observations_and_marks_elapsed_resets() {
    let now = Utc::now();
    let mut fresh = AccountRecord::new(Provider::Codex, "fresh");
    fresh.quota = Some(QuotaSnapshot {
        windows: vec![
            QuotaWindow {
                kind: QuotaWindowKind::Weekly,
                label: "Weekly".into(),
                used_percent: 70,
                resets_at: Some(now - ChronoDuration::seconds(1)),
                window_duration_minutes: Some(10080),
                detail: None,
            },
            QuotaWindow {
                kind: QuotaWindowKind::WeeklyModel,
                label: "Model".into(),
                used_percent: 100,
                resets_at: None,
                window_duration_minutes: Some(10080),
                detail: None,
            },
        ],
        ..QuotaSnapshot::default()
    });
    fresh.last_updated = Some(now - ChronoDuration::minutes(30));
    let mut stale = fresh.clone();
    stale.slug = "stale".into();
    stale.last_updated = Some(now - ChronoDuration::hours(3));

    let selected = select_account(
        &[stale, fresh],
        Provider::Codex,
        SelectionOptions::default(),
        now,
    )
    .unwrap();
    assert_eq!(selected.account.slug, "fresh");
    assert_eq!(selected.available_percent, 100);
    assert_eq!(selected.freshness, Freshness::Fresh);
    assert!(selected.projected);
}

#[tokio::test]
async fn a_reserved_slug_cannot_be_reused_while_an_abandoned_login_is_recovered() {
    use fs2::FileExt;

    let scratch = TempDir::new().unwrap();
    let backend = Arc::new(FakeProvider::new(Provider::Codex));
    let manager = fake_manager(scratch.path(), backend);
    manager.prepare().await.unwrap();

    let record = AccountRecord::new(Provider::Codex, "work");
    let vault = &manager.config().vault_root;
    let recovery_lock =
        fs::File::create(vault.join(format!(".locks/account-{}", record.id))).unwrap();
    FileExt::lock_exclusive(&recovery_lock).unwrap();
    let journal = vault.join(format!(".pending/{}.operation.json", record.id));
    fs::write(
        &journal,
        serde_json::json!({
            "account": record, "kind": {"operation":"add", "from_default": false}
        })
        .to_string(),
    )
    .unwrap();

    let rejected = manager
        .add(
            Provider::Codex,
            options("work", "owner@example.test"),
            progress(),
        )
        .await;
    assert!(matches!(rejected, Err(Error::SlugTaken(_))));
    drop(recovery_lock);

    manager.prepare().await.unwrap();
    assert!(!journal.exists());
    assert!(
        manager
            .add(
                Provider::Codex,
                options("work", "owner@example.test"),
                progress()
            )
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn reconnect_preserves_old_config_and_colliding_history_directories() {
    let scratch = TempDir::new().unwrap();
    let manager = fake_manager(scratch.path(), Arc::new(FakeProvider::new(Provider::Codex)));
    let record = manager
        .add(
            Provider::Codex,
            options("work", "owner@example.test"),
            progress(),
        )
        .await
        .unwrap();
    let home = manager.home(&record);
    fs::create_dir_all(home.join("sessions")).unwrap();
    fs::write(home.join("sessions/old"), "history").unwrap();
    fs::write(home.join("config.toml"), "user config").unwrap();
    let result = manager
        .reconnect("codex/work", progress(), false)
        .await
        .unwrap();

    assert_eq!(result.id, record.id);
    assert_eq!(
        fs::read_to_string(home.join("sessions/old")).unwrap(),
        "history"
    );
    assert_eq!(
        fs::read_to_string(home.join("sessions/fresh")).unwrap(),
        "new login state"
    );
    assert_eq!(
        fs::read_to_string(home.join("config.toml")).unwrap(),
        "user config"
    );
    assert_eq!(
        fs::read_to_string(home.join("auth.json")).unwrap(),
        "owner@example.test"
    );
}

#[tokio::test]
async fn a_removed_shared_setting_link_stays_opted_out() {
    let scratch = TempDir::new().unwrap();
    let manager = fake_manager(scratch.path(), Arc::new(FakeProvider::new(Provider::Codex)));
    let default = manager.config().user_home.join(".codex");
    fs::create_dir_all(&default).unwrap();
    fs::write(default.join("AGENTS.md"), "shared").unwrap();

    let record = manager
        .add(
            Provider::Codex,
            options("work", "owner@example.test"),
            progress(),
        )
        .await
        .unwrap();
    let link = manager.home(&record).join("AGENTS.md");
    assert_eq!(fs::read_link(&link).unwrap(), default.join("AGENTS.md"));
    fs::remove_file(&link).unwrap();
    manager.refresh(Some("codex/work")).await.unwrap();
    assert!(!link.exists());

    manager
        .reconnect("codex/work", progress(), false)
        .await
        .unwrap();
    assert!(!link.exists());
    assert!(fs::read_to_string(default.join("AGENTS.md")).is_ok());
}

#[tokio::test]
async fn manifest_paths_and_values_survive_a_current_write() {
    let scratch = TempDir::new().unwrap();
    let backend = Arc::new(FakeProvider::new(Provider::Claude));
    let manager = fake_manager(scratch.path(), backend.clone());
    let vault = &manager.config().vault_root;
    let path = vault.join("accounts.json");
    let home = vault.join("claude/work");
    fs::create_dir_all(&home).unwrap();
    let id = Uuid::new_v4();
    let original = serde_json::json!({
        "version": 3,
        "root": vault,
        "accounts": [{
            "id": id.to_string().to_uppercase(), "provider": "claude", "slug": "work",
            "email": "owner@example.test", "reference": "claude/work", "home": home,
            "env": { "CLAUDE_CONFIG_DIR": home },
            "lastUpdated": "2024-11-18T20:25:30Z",
            "quota": { "windows": [{
                "kind": "weeklyModel", "label": "Model", "usedPercent": 36,
                "resetsAt": "2024-11-19T00:00:00Z", "windowDurationMinutes": 10080
            }], "resetCredits": { "available": 0 } }
        }]
    });
    fs::write(&path, original.to_string()).unwrap();

    let account = manager.resolve("owner@example.test").await.unwrap();
    assert_eq!(account.id, id);
    assert_eq!(account.quota.unwrap().reset_credits.unwrap().available, 0);
    backend
        .credentials
        .lock()
        .unwrap()
        .insert(home.clone(), "owner@example.test".into());
    let report = manager.refresh(Some("claude/work")).await.unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let updated: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(updated["version"], 3);
    assert_eq!(updated["accounts"][0]["home"], home.to_str().unwrap());
    assert_eq!(
        updated["accounts"][0]["env"]["CLAUDE_CONFIG_DIR"],
        home.to_str().unwrap()
    );
    let timestamp = updated["accounts"][0]["lastUpdated"].as_str().unwrap();
    assert!(timestamp.ends_with('Z') && !timestamp.contains('.'));

    for version in [1, 2, 42] {
        let mut unsupported = original.clone();
        unsupported["version"] = version.into();
        let unsupported = unsupported.to_string();
        fs::write(&path, &unsupported).unwrap();
        let error = manager
            .add(
                Provider::Claude,
                options("another", "another@example.test"),
                progress(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::UnsupportedManifest(value) if value == version));
        assert_eq!(fs::read_to_string(&path).unwrap(), unsupported);
    }
}
