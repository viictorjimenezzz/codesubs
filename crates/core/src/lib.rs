//! Manage subscription accounts through an isolated, coordinated credential vault.

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
pub use manager::{DoctorReport, ProviderDiagnostic, SubscriptionManager};
pub use models::*;
pub use providers::{CredentialLocation, ProviderBackend};
pub use services::selection::select_account;
