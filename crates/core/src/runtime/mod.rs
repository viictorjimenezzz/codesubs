#[cfg(unix)]
mod login_codes;
mod process;

#[cfg(unix)]
pub(crate) use login_codes::LoginCodes;
#[cfg(unix)]
pub use login_codes::submit_login_code;
pub(crate) use process::{CommandProcess, ProcessOutput, create_private_dir, run, wait_for_cancel};
pub use process::{configure_account, locate_executable};

#[cfg(not(unix))]
pub fn submit_login_code(
    _pending_directory: &std::path::Path,
    _target: Option<&str>,
    _code: &str,
) -> crate::Result<()> {
    Err(crate::Error::Unsupported(
        "Local sign-in code submission requires Unix.".into(),
    ))
}
