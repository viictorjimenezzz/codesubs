//! Manage Codex and Claude Code subscription accounts with codesubs.

mod config;
mod error;
mod manager;
mod models;
mod platform;
pub mod providers;
mod runtime;
mod services;
mod storage;

pub use config::ManagerConfig;
pub use error::{Error, Result};
pub use manager::{Codesubs, DoctorReport, ProviderDiagnostic};
pub use models::*;
pub use providers::{CredentialLocation, ProviderBackend};
pub use services::selection::select_account;

/// Compatibility name for the account manager.
pub type SubscriptionManager = Codesubs;
