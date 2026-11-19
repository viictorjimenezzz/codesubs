use crate::{
    args::{Args, Command},
    output, shell,
};
use codesubs_core::{
    AddOptions, Codesubs, LoginEvent, LoginProgress, ManagerConfig, Provider, SelectionOptions,
};
use serde_json::json;
use std::{
    io::{IsTerminal, Read},
    sync::Arc,
    time::Duration,
};

pub struct Failure {
    pub code: String,
    pub message: String,
}
impl Failure {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_input".into(),
            message: message.into(),
        }
    }
}
impl From<codesubs_core::Error> for Failure {
    fn from(error: codesubs_core::Error) -> Self {
        Self {
            code: error.code().into(),
            message: error.to_string(),
        }
    }
}

fn parse_provider(value: &str) -> Result<Provider, Failure> {
    value
        .parse()
        .map_err(|_| Failure::invalid(format!("unknown provider {value:?}; use codex or claude")))
}

fn progress(
    events: bool,
    reference: Option<String>,
    provider: Provider,
    cancel: tokio::sync::watch::Sender<bool>,
) -> LoginProgress {
    let login_id = uuid::Uuid::new_v4().to_string();
    Arc::new(move |event| match event {
        LoginEvent::SignInUrl { url } => {
            if events {
                let written = output::json::event(
                    &json!({"protocolVersion":1,"type":"login","event":"login","loginID":login_id,"authURL":url,"browserOpened":provider==Provider::Claude,"acceptsCode":provider==Provider::Claude,"reference":reference}),
                );
                if !written {
                    let _ = cancel.send(true);
                }
            } else {
                eprintln!("Open this URL to finish signing in:\n{url}");
            }
        }
        LoginEvent::WaitingForCode { slug } => {
            if events {
                let written = output::json::event(
                    &json!({"protocolVersion":1,"type":"waitingForCode","event":"waitingForCode","slug":slug,"loginID":login_id,"reference":reference}),
                );
                if !written {
                    let _ = cancel.send(true);
                }
            } else {
                eprintln!("If a code is shown, run: codesubs add-code --slug {slug} '<code>'");
            }
        }
    })
}

type Cancellation = (
    tokio::sync::watch::Sender<bool>,
    tokio::sync::watch::Receiver<bool>,
    tokio::task::JoinHandle<()>,
);

fn cancellation() -> Result<Cancellation, Failure> {
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let notify = sender.clone();
    #[cfg(unix)]
    let (mut interrupt, mut term) = {
        use tokio::signal::unix::{SignalKind, signal};
        (
            signal(SignalKind::interrupt()).map_err(|e| Failure::invalid(e.to_string()))?,
            signal(SignalKind::terminate()).map_err(|e| Failure::invalid(e.to_string()))?,
        )
    };
    let task = tokio::spawn(async move {
        #[cfg(unix)]
        {
            tokio::select! { _ = interrupt.recv() => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = notify.send(true);
    });
    Ok((sender, receiver, task))
}

pub async fn run(args: Args) -> Result<(), Failure> {
    if args.json_events
        && !matches!(
            &args.command,
            Command::Add { .. } | Command::Reconnect { .. }
        )
    {
        return Err(Failure::invalid(
            "--json-events is only supported by add and reconnect",
        ));
    }
    if args.protocol_version != 1 {
        return Err(Failure::invalid(
            "unsupported protocol version; supported: 1",
        ));
    }
    if matches!(args.command, Command::Version) {
        if args.json {
            output::json::print(&json!({"protocolVersion":1,"version":env!("CARGO_PKG_VERSION")}));
        } else {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }
        return Ok(());
    }
    let config = match args.home {
        Some(vault_root) => {
            let user_home = std::env::home_dir().ok_or_else(|| {
                Failure::invalid("could not locate the current user's home directory")
            })?;
            ManagerConfig::new(vault_root, user_home)?
        }
        None => ManagerConfig::from_environment()?,
    };
    let manager = Codesubs::new(config)?;
    match args.command {
        Command::List { provider } => {
            let provider = provider.as_deref().map(parse_provider).transpose()?;
            let accounts = manager
                .list()
                .await?
                .into_iter()
                .filter(|a| provider.as_ref().is_none_or(|p| &a.provider == p))
                .collect::<Vec<_>>();
            if args.json {
                output::json::print(&json!(
                    accounts
                        .iter()
                        .map(|a| output::json::account(&manager, a))
                        .collect::<Vec<_>>()
                ));
            } else {
                output::table::accounts(&accounts);
            }
        }
        Command::Info => {
            manager.prepare().await?;
            let root = &manager.config().vault_root;
            let info = json!({"protocolVersion":1,"version":env!("CARGO_PKG_VERSION"),"root":root,"guidePath":root.join("README.md"),"addGuidePath":root.join("ADD.md")});
            if args.json {
                output::json::print(&info);
            } else {
                println!(
                    "{}\nVault: {}\nProtocol: 1",
                    env!("CARGO_PKG_VERSION"),
                    root.display()
                );
            }
        }
        Command::Refresh { account } => {
            let report = manager.refresh(account.as_deref()).await?;
            if args.json {
                output::json::print(
                    &json!({"protocolVersion":1,"accounts":report.accounts.iter().map(|a| output::json::account(&manager,a)).collect::<Vec<_>>(),"failures":report.failures}),
                );
            } else {
                output::table::accounts(&report.accounts);
                for failure in report.failures {
                    eprintln!("{}: {}", failure.reference, failure.message);
                }
            }
        }
        Command::Pick {
            provider,
            minimum,
            max_age,
            refresh,
        } => {
            if !(0..=100).contains(&minimum) {
                return Err(Failure::invalid("--min must be between 0 and 100"));
            }
            let provider = parse_provider(&provider)?;
            let options = SelectionOptions {
                minimum_percent: minimum,
                max_age: Duration::from_secs(max_age.unwrap_or(3600)),
                require_fresh: max_age.is_some() || refresh,
            };
            let selection = if refresh {
                let report = manager.refresh_provider(provider).await?;
                for failure in &report.failures {
                    eprintln!("{}: {}", failure.reference, failure.message);
                }
                codesubs_core::select_account(
                    &report.accounts,
                    provider,
                    options,
                    chrono::Utc::now(),
                )?
            } else {
                manager.pick(provider, options).await?
            };
            if args.json {
                output::json::print(
                    &json!({"protocolVersion":1,"account":output::json::account(&manager,&selection.account),"availablePercent":selection.available_percent,"freshness":selection.freshness,"projected":selection.projected}),
                );
            } else {
                println!("{}", selection.account.reference());
            }
        }
        Command::Env { account, fish, sh } => {
            let context = manager.env(&account).await?;
            let fish = fish
                || (!sh
                    && std::env::var("SHELL").is_ok_and(|s| {
                        std::path::Path::new(&s)
                            .file_name()
                            .is_some_and(|n| n == "fish")
                    }));
            if args.json {
                output::json::print(
                    &json!({"protocolVersion":1,"home":context.home,"set":context.set,"unset":context.unset}),
                );
            } else {
                println!("{}", shell::environment(&context.set, &context.unset, fish));
            }
        }
        Command::Path { account } => {
            let context = manager.env(&account).await?;
            if args.json {
                output::json::print(&json!({"protocolVersion":1,"home":context.home}));
            } else {
                println!("{}", context.home.display());
            }
        }
        Command::Exec { account, command } => {
            let context = manager.env(&account).await?;
            let Some(executable) = command.first() else {
                return Err(Failure::invalid("exec needs a command"));
            };
            let mut process = std::process::Command::new(executable);
            process.args(&command[1..]);
            for key in context.unset {
                process.env_remove(key);
            }
            process.envs(context.set);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let error = process.exec();
                return Err(Failure {
                    code: "executable_not_found".into(),
                    message: format!("could not run {}: {error}", executable.to_string_lossy()),
                });
            }
            #[cfg(not(unix))]
            {
                let status = process
                    .status()
                    .map_err(|e| Failure::invalid(e.to_string()))?;
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Command::Add {
            provider,
            email,
            slug,
            from_default,
        } => {
            let provider = parse_provider(&provider)?;
            let (cancel_sender, cancel, signal_task) = cancellation()?;
            let result = manager
                .add(
                    provider,
                    AddOptions {
                        expected_email: email,
                        slug,
                        from_default,
                        accept_terminal_input: !args.json_events && std::io::stdin().is_terminal(),
                        cancel: Some(cancel),
                    },
                    progress(args.json_events, None, provider, cancel_sender),
                )
                .await;
            signal_task.abort();
            let account = result?;
            completed(&manager, &account, args.json, args.json_events);
        }
        Command::Reconnect { account } => {
            let existing = manager.resolve(&account).await?;
            let (cancel_sender, cancel, signal_task) = cancellation()?;
            let result = manager
                .reconnect_with_cancellation(
                    &account,
                    progress(
                        args.json_events,
                        Some(existing.reference()),
                        existing.provider,
                        cancel_sender,
                    ),
                    !args.json_events && std::io::stdin().is_terminal(),
                    Some(cancel),
                )
                .await;
            signal_task.abort();
            let account = result?;
            completed(&manager, &account, args.json, args.json_events);
        }
        Command::AddCode { code, stdin, slug } => {
            let code = if stdin {
                let mut input = String::new();
                std::io::stdin()
                    .read_to_string(&mut input)
                    .map_err(|e| Failure::invalid(e.to_string()))?;
                input
            } else {
                code.unwrap_or_default()
            };
            manager.submit_code(code.trim(), slug.as_deref()).await?;
            if args.json {
                output::json::print(&json!({"protocolVersion":1,"sent":true}));
            } else {
                println!("Code sent. The login command finishes on its own.");
            }
        }
        Command::Rename { account, slug } => {
            let account = manager.rename(&account, &slug).await?;
            if args.json {
                output::json::print(
                    &json!({"protocolVersion":1,"account":output::json::account(&manager,&account)}),
                );
            } else {
                println!("Renamed account to {}", account.reference());
            }
        }
        Command::Remove { account, yes } => {
            if !yes {
                return Err(Failure::invalid(
                    "removal deletes the account login and folder; pass --yes to confirm",
                ));
            }
            let account = manager.remove(&account).await?;
            if args.json {
                output::json::print(&json!({"protocolVersion":1,"removed":account.reference()}));
            } else {
                println!("Removed {}", account.reference());
            }
        }
        Command::Reset {
            account,
            yes,
            idempotency_key,
        } => {
            if yes {
                let receipt = manager.redeem_reset(&account, idempotency_key).await?;
                let raw = serde_json::to_value(&receipt.outcome)
                    .map_err(|e| Failure::invalid(e.to_string()))?;
                let message = reset_message(raw.as_str().unwrap_or("unknown"));
                if args.json {
                    output::json::print(
                        &json!({"protocolVersion":1,"outcome":raw,"message":message,"account":output::json::account(&manager,&receipt.account),"idempotencyKey":receipt.idempotency_key}),
                    );
                } else {
                    println!("{}: {message}", receipt.account.reference());
                }
            } else {
                let preview = manager.preview_reset(&account).await?;
                if args.json {
                    let mut value = serde_json::to_value(&preview)
                        .map_err(|e| Failure::invalid(e.to_string()))?;
                    value
                        .as_object_mut()
                        .unwrap()
                        .insert("protocolVersion".into(), json!(1));
                    output::json::print(&value);
                } else {
                    println!(
                        "{}: {} banked resets, {}% of the week left.{}",
                        preview.account.reference(),
                        preview
                            .available
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        preview
                            .weekly_remaining_percent
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        if preview.can_redeem {
                            " Re-run with --yes to apply one after the user agrees."
                        } else {
                            " Redeem Claude resets in Claude Settings > Usage."
                        }
                    );
                }
            }
        }
        Command::Doctor => {
            let report = manager.doctor().await?;
            let mut value =
                serde_json::to_value(report).map_err(|e| Failure::invalid(e.to_string()))?;
            value
                .as_object_mut()
                .unwrap()
                .insert("protocolVersion".into(), json!(1));
            output::json::print(&value);
        }
        Command::Version => unreachable!(),
    }
    Ok(())
}

fn completed(
    manager: &Codesubs,
    account: &codesubs_core::AccountRecord,
    machine: bool,
    events: bool,
) {
    let value = json!({"protocolVersion":1,"type":"completed","event":"complete","account":output::json::account(manager,account)});
    if events {
        let _ = output::json::event(&value);
    } else if machine {
        output::json::print(&value);
    } else {
        println!(
            "Connected {} ({}, {})",
            account.reference(),
            account.email.as_deref().unwrap_or("unknown email"),
            account.plan.as_deref().unwrap_or("unknown plan")
        );
    }
}

fn reset_message(outcome: &str) -> &str {
    match outcome {
        "reset" => "Reset applied.",
        "nothingToReset" => "Nothing to reset; no credit was used.",
        "noCredit" => "No banked resets are available.",
        "alreadyRedeemed" => "This reset was already applied.",
        _ => "Provider returned an unrecognized reset outcome.",
    }
}
