use clap::{Parser, Subcommand};
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "codesubs",
    version,
    about = "Manage subscription accounts independently of the macOS app"
)]
pub struct Args {
    /// Vault directory (also CODESUBS_HOME).
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,
    /// Emit documented machine-readable output.
    #[arg(long, global = true)]
    pub json: bool,
    /// Emit login progress as JSON Lines.
    #[arg(long, global = true)]
    pub json_events: bool,
    /// Machine protocol version.
    #[arg(long, global = true, default_value_t = 1)]
    pub protocol_version: u32,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show cached accounts and quota.
    List { provider: Option<String> },
    /// Read current provider quota; without an account refresh all accounts.
    Refresh { account: Option<String> },
    /// Pick an account with the most available quota.
    Pick {
        provider: String,
        #[arg(long = "min", default_value_t = 1)]
        minimum: i64,
        #[arg(long)]
        max_age: Option<u64>,
        #[arg(long)]
        refresh: bool,
    },
    /// Print an account's shell environment.
    Env {
        account: String,
        #[arg(long, conflicts_with = "sh")]
        fish: bool,
        #[arg(long)]
        sh: bool,
        /// Agent for an API account: claude (default) or codex.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Print the exact account home.
    Path {
        account: String,
        /// Agent for an API account: claude (default) or codex.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Replace this process with a command using an account.
    Exec {
        account: String,
        /// Agent for an API account; inferred from a `claude` or `codex` command.
        #[arg(long)]
        agent: Option<String>,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Manage API-key accounts such as Melious.
    Api {
        #[command(subcommand)]
        command: ApiCommand,
    },
    /// Sign in a new account, verifying its email before registration.
    Add {
        provider: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        slug: Option<String>,
        #[arg(long)]
        from_default: bool,
    },
    /// Sign in again inside an existing account home.
    Reconnect { account: String },
    /// Send an authorization code to a running login (use --stdin to avoid argv).
    AddCode {
        #[arg(required_unless_present = "stdin", conflicts_with = "stdin")]
        code: Option<String>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        slug: Option<String>,
    },
    /// Rename an account home with credential verification.
    Rename { account: String, slug: String },
    /// Remove an account and its credentials.
    Remove {
        account: String,
        #[arg(long)]
        yes: bool,
    },
    /// Preview banked resets, or explicitly redeem a Codex reset.
    Reset {
        account: String,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        idempotency_key: Option<uuid::Uuid>,
    },
    /// Inspect local provider capabilities and vault health.
    Doctor,
    /// Print installation and protocol metadata.
    Info,
    /// Print the version.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum ApiCommand {
    /// Save an API key read from stdin and verify it works.
    Add {
        service: String,
        #[arg(long)]
        slug: Option<String>,
        /// Read the key from stdin so it never appears in argv.
        #[arg(long)]
        key_stdin: bool,
        /// Anthropic Messages API root (Claude Code).
        #[arg(long)]
        anthropic_url: Option<String>,
        /// OpenAI API base including /v1 (Codex).
        #[arg(long)]
        openai_url: Option<String>,
        /// Model for Claude's Opus/Sonnet aliases and Codex.
        #[arg(long)]
        big: Option<String>,
        /// Model for Claude's Haiku alias and background tasks.
        #[arg(long)]
        small: Option<String>,
        /// Save without a health check.
        #[arg(long)]
        no_check: bool,
    },
    /// Show API accounts and their last health check.
    List,
    /// Check that keys and models work; without an account check all.
    Check { account: Option<String> },
    /// Choose the big and small models.
    SetModels {
        account: String,
        #[arg(long)]
        big: Option<String>,
        #[arg(long)]
        small: Option<String>,
    },
    /// Remove an API account and its stored key.
    Remove {
        account: String,
        #[arg(long)]
        yes: bool,
    },
}
