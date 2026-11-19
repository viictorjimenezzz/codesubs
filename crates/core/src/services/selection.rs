use chrono::{DateTime, Utc};

use crate::{
    AccountRecord, Error, Freshness, Provider, QuotaWindowKind, Result, Selection, SelectionOptions,
};

pub(crate) fn resolve(accounts: &[AccountRecord], reference: &str) -> Result<AccountRecord> {
    let needle = reference.to_lowercase();
    if let Some(exact) = accounts
        .iter()
        .find(|account| account.reference() == needle)
    {
        return Ok(exact.clone());
    }

    let matches: Vec<_> = accounts
        .iter()
        .filter(|account| {
            account.slug == needle
                || account
                    .email
                    .as_ref()
                    .is_some_and(|email| email.to_lowercase() == needle)
        })
        .collect();
    match matches.as_slice() {
        [] => Err(Error::AccountNotFound(reference.into())),
        [one] => Ok((*one).clone()),
        many => Err(Error::AmbiguousReference {
            reference: reference.into(),
            matches: many
                .iter()
                .map(|account| account.reference())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

pub fn select_account(
    accounts: &[AccountRecord],
    provider: Provider,
    options: SelectionOptions,
    now: DateTime<Utc>,
) -> Result<Selection> {
    if !(0..=100).contains(&options.minimum_percent) {
        return Err(Error::InvalidInput(
            "minimum quota must be between 0 and 100".into(),
        ));
    }

    let mut candidates: Vec<_> = accounts
        .iter()
        .filter_map(|account| {
            if account.provider != provider || account.is_provisional() {
                return None;
            }
            let quota = account.quota.as_ref()?;
            let available_percent = quota.available_percent(now)?;
            let freshness = match account.last_updated {
                None => Freshness::Unknown,
                Some(updated)
                    if now
                        .signed_duration_since(updated)
                        .to_std()
                        .is_ok_and(|age| age < options.max_age) =>
                {
                    Freshness::Fresh
                }
                Some(_) => Freshness::Stale,
            };
            if available_percent < options.minimum_percent
                || (options.require_fresh && freshness != Freshness::Fresh)
            {
                return None;
            }
            let weekly = quota
                .weekly()
                .map_or(available_percent, |window| window.remaining_percent_at(now));
            let projected = quota.windows.iter().any(|window| {
                window.kind != QuotaWindowKind::WeeklyModel
                    && window.remaining_percent() != window.remaining_percent_at(now)
            });
            Some((
                Selection {
                    account: account.clone(),
                    available_percent,
                    freshness,
                    projected,
                },
                weekly,
            ))
        })
        .collect();

    candidates.sort_by_key(|(result, weekly)| {
        std::cmp::Reverse((
            result.freshness == Freshness::Fresh,
            result.available_percent,
            *weekly,
        ))
    });
    candidates
        .into_iter()
        .next()
        .map(|(selection, _)| selection)
        .ok_or(Error::NoAvailableAccount {
            provider,
            minimum: options.minimum_percent,
        })
}
