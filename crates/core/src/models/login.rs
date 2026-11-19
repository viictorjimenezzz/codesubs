use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[derive(Clone, Debug, Default)]
pub struct AddOptions {
    pub expected_email: Option<String>,
    pub slug: Option<String>,
    pub from_default: bool,
    pub accept_terminal_input: bool,
    pub cancel: Option<watch::Receiver<bool>>,
}

#[derive(Clone, Debug)]
pub struct LoginRequest {
    pub home: PathBuf,
    pub expected_email: Option<String>,
    pub pending_directory: PathBuf,
    pub slug: String,
    pub accept_terminal_input: bool,
    pub cancel: Option<watch::Receiver<bool>>,
}

pub type LoginProgress = Arc<dyn Fn(LoginEvent) + Send + Sync>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum LoginEvent {
    SignInUrl { url: String },
    WaitingForCode { slug: String },
}
