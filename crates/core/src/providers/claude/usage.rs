use std::sync::LazyLock;

use chrono::{
    DateTime, Datelike, Days, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Utc,
};
use chrono_tz::Tz;
use regex::Regex;

use crate::{QuotaSnapshot, QuotaWindow, QuotaWindowKind};

static USAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^Current (session|week)(?: \(([^)]*)\))?:\s*([0-9]+(?:\.[0-9]+)?)%\s+used(?:\s*[·•-]\s*resets\s+(.+?))?\s*$")
        .expect("valid /usage expression")
});
static RESET_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:([A-Za-z]{3,9})\.?\s+([0-9]{1,2})(?:,\s*([0-9]{4}))?\s+at\s+)?([0-9]{1,2})(?::([0-9]{2}))?\s*(am|pm)?$")
        .expect("valid reset time expression")
});
static TIME_ZONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\(([^()]+)\)\s*$").expect("valid timezone expression"));

pub fn quota(text: &str, now: DateTime<Utc>) -> Option<QuotaSnapshot> {
    let windows: Vec<_> = text
        .lines()
        .filter_map(|line| window(line.trim(), now))
        .collect();
    if windows.is_empty() {
        return None;
    }
    Some(QuotaSnapshot {
        windows,
        ..QuotaSnapshot::default()
    })
}

pub fn window(line: &str, now: DateTime<Utc>) -> Option<QuotaWindow> {
    let matched = USAGE.captures(line)?;
    let percent = matched[3].parse::<f64>().ok()?.round();
    if percent >= -(i64::MIN as f64) {
        return None;
    }
    let qualifier = matched.get(2).map(|matched| matched.as_str());
    let (kind, label, duration) = if matched[1].eq_ignore_ascii_case("session") {
        (QuotaWindowKind::Session, "5h", 300)
    } else if let Some(model) =
        qualifier.filter(|qualifier| !qualifier.eq_ignore_ascii_case("all models"))
    {
        (QuotaWindowKind::WeeklyModel, model, 10_080)
    } else {
        (QuotaWindowKind::Weekly, "Week", 10_080)
    };
    Some(QuotaWindow {
        kind,
        label: label.to_owned(),
        used_percent: percent as i64,
        resets_at: matched
            .get(4)
            .and_then(|matched| reset_date(matched.as_str(), now)),
        window_duration_minutes: Some(duration),
        detail: None,
    })
}

pub fn reset_date(text: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let text = text.trim();
    let (time, zone) = match TIME_ZONE.captures(text) {
        Some(matched) => (
            &text[..matched.get(0)?.start()],
            Zone::from_name(&matched[1]),
        ),
        None => (text, Zone::Local),
    };
    let matched = RESET_TIME.captures(time)?;
    let mut hour = matched[4].parse::<u32>().ok()?;
    let minute = matched
        .get(5)
        .map_or(Some(0), |minute| minute.as_str().parse().ok())?;
    if let Some(half_day) = matched.get(6) {
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour %= 12;
        if half_day.as_str().eq_ignore_ascii_case("pm") {
            hour += 12;
        }
    }

    let today = zone.today(now);
    if let Some(month) = matched.get(1) {
        let month = month_number(month.as_str())?;
        let day = matched[2].parse().ok()?;
        let explicit_year = matched.get(3).and_then(|year| year.as_str().parse().ok());
        let year = explicit_year.unwrap_or(today.year());
        let date = NaiveDate::from_ymd_opt(year, month, day)?;
        let reset = zone.utc(&date.and_hms_opt(hour, minute, 0)?)?;
        if explicit_year.is_none() && now.signed_duration_since(reset) > Duration::days(1) {
            let next = NaiveDate::from_ymd_opt(year + 1, month, day)?;
            return zone.utc(&next.and_hms_opt(hour, minute, 0)?);
        }
        return Some(reset);
    }

    let reset = zone.utc(&today.and_hms_opt(hour, minute, 0)?)?;
    if reset < now {
        let tomorrow = today.checked_add_days(Days::new(1))?;
        zone.utc(&tomorrow.and_hms_opt(hour, minute, 0)?)
    } else {
        Some(reset)
    }
}

fn month_number(name: &str) -> Option<u32> {
    let prefix = name.get(..3)?.to_ascii_lowercase();
    [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|month| *month == prefix)
    .map(|position| position as u32 + 1)
}

enum Zone {
    Local,
    Named(Tz),
    Fixed(FixedOffset),
}

impl Zone {
    fn from_name(name: &str) -> Self {
        if let Ok(zone) = name.parse::<Tz>() {
            return Self::Named(zone);
        }
        let hours = match name {
            "UTC" | "GMT" => 0,
            "CET" => 1,
            "CEST" | "EET" => 2,
            "EEST" => 3,
            "EST" | "CDT" => -5,
            "EDT" => -4,
            "CST" | "MDT" => -6,
            "MST" | "PDT" => -7,
            "PST" => -8,
            "JST" => 9,
            _ => return Self::Local,
        };
        Self::Fixed(FixedOffset::east_opt(hours * 3600).expect("valid UTC offset"))
    }

    fn today(&self, now: DateTime<Utc>) -> NaiveDate {
        match self {
            Self::Local => now.with_timezone(&Local).date_naive(),
            Self::Named(zone) => now.with_timezone(zone).date_naive(),
            Self::Fixed(zone) => now.with_timezone(zone).date_naive(),
        }
    }

    fn utc(&self, time: &NaiveDateTime) -> Option<DateTime<Utc>> {
        match self {
            Self::Local => Local
                .from_local_datetime(time)
                .earliest()
                .map(|date| date.to_utc()),
            Self::Named(zone) => zone
                .from_local_datetime(time)
                .earliest()
                .map(|date| date.to_utc()),
            Self::Fixed(zone) => zone
                .from_local_datetime(time)
                .earliest()
                .map(|date| date.to_utc()),
        }
    }
}
