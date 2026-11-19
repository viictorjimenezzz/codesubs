mod journal;

mod homes;
mod locks;
mod manifest;
mod migrations;
mod repository;
mod shared_config;

pub(crate) use homes::{exists, move_directory, remove_directory, require_directory};
pub(crate) use journal::{Journal, Operation};

pub(crate) use manifest::Manifest;
pub(crate) use repository::Repository;

pub(crate) fn validate_slug(slug: &str) -> crate::Result<()> {
    if slug.is_empty() || sanitize_slug(slug) != slug {
        return Err(crate::Error::InvalidSlug(slug.into()));
    }
    Ok(())
}

pub(crate) fn slug_from_email(email: &str) -> String {
    let mut parts = email.splitn(2, '@');
    let username = parts.next().unwrap_or_default();
    let domain = parts
        .next()
        .and_then(|parts| parts.split('.').find(|part| !part.is_empty()));
    sanitize_slug(&domain.map_or_else(|| username.into(), |domain| format!("{username}-{domain}")))
}

pub(crate) fn sanitize_slug(text: &str) -> String {
    let mut result = String::new();
    let mut dash = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            if dash {
                result.push('-');
                dash = false;
            }
            result.push(character);
        } else if !result.is_empty() {
            dash = true;
        }
    }
    result.truncate(48);
    result.trim_end_matches('-').into()
}
