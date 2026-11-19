use uuid::Uuid;

use crate::Provider;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidInput(String),
    #[error("invalid account slug {0:?}, use up to 48 lowercase letters, digits and single dashes")]
    InvalidSlug(String),
    #[error("no account found for {0:?}")]
    AccountNotFound(String),
    #[error("{reference:?} matches {matches}, use a provider/slug reference")]
    AmbiguousReference { reference: String, matches: String },
    #[error("no {provider} account has at least {minimum}% eligible quota available")]
    NoAvailableAccount { provider: Provider, minimum: i64 },
    #[error("account path {0} is already taken")]
    SlugTaken(String),
    #[error("{email} is already registered as {reference}")]
    DuplicateIdentity { email: String, reference: String },
    #[error("expected {expected}, provider reported {actual}")]
    IdentityMismatch { expected: String, actual: String },
    #[error("{0}")]
    Unsupported(String),
    #[error("invalid account manifest: {0}")]
    CorruptManifest(String),
    #[error("account manifest version {0} is unsupported")]
    UnsupportedManifest(u32),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    Login(String),
    #[error("could not locate the {0} executable")]
    ExecutableNotFound(Provider),
    #[error("operation cancelled")]
    Cancelled,
    #[error("account {0} changed during the operation")]
    StaleState(String),
    #[error("{0}")]
    ResetUnavailable(String),
    #[error("reset result could not be confirmed. Retry with idempotency key {key}. {reason}")]
    ResetUncertain { key: Uuid, reason: String },
    #[error("{0}")]
    RecoveryRequired(String),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidSlug(_) => "invalid_slug",
            Self::AccountNotFound(_) => "account_not_found",
            Self::AmbiguousReference { .. } => "ambiguous_reference",
            Self::NoAvailableAccount { .. } => "no_available_account",
            Self::SlugTaken(_) => "slug_taken",
            Self::DuplicateIdentity { .. } => "duplicate_identity",
            Self::IdentityMismatch { .. } => "identity_mismatch",
            Self::Unsupported(_) => "unsupported",
            Self::CorruptManifest(_) => "corrupt_manifest",
            Self::UnsupportedManifest(_) => "unsupported_manifest",
            Self::Io(_) => "io",
            Self::Provider(_) => "provider_error",
            Self::Timeout(_) => "timeout",
            Self::Login(_) => "login_error",
            Self::ExecutableNotFound(_) => "executable_not_found",
            Self::Cancelled => "cancelled",
            Self::StaleState(_) => "stale_state",
            Self::ResetUnavailable(_) => "reset_unavailable",
            Self::ResetUncertain { .. } => "reset_uncertain",
            Self::RecoveryRequired(_) => "recovery_required",
        }
    }
}
