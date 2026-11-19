use chrono::Utc;
use codesubs_core::{AccountRecord, Codesubs};
use serde_json::{Value, json};

pub fn account(manager: &Codesubs, record: &AccountRecord) -> Value {
    let now = Utc::now();
    let mut value = serde_json::to_value(record).expect("account serialization");
    let object = value.as_object_mut().expect("account object");
    let home = manager.home(record).to_string_lossy().into_owned();
    let variable = record.provider.home_environment_variable();
    object.insert("reference".into(), json!(record.reference()));
    object.insert("home".into(), json!(home));
    object.insert("env".into(), json!({ variable: home }));
    if let Some(quota) = &record.quota {
        if let Some(percent) = quota.available_percent(now) {
            object.insert("availablePercent".into(), json!(percent));
        }
        if let Some(weekly) = quota.weekly() {
            object.insert(
                "weeklyRemainingPercent".into(),
                json!(weekly.remaining_percent_at(now)),
            );
        }
        if let Some(credits) = &quota.reset_credits
            && let Some(aggregate) = object
                .get_mut("quota")
                .and_then(|q| q.get_mut("resetCredits"))
                .and_then(Value::as_object_mut)
        {
            aggregate.insert("available".into(), json!(credits.available_at(now)));
            aggregate.insert("nextExpiry".into(), json!(credits.next_expiry_at(now)));
        }
    }
    value
}

pub fn print(value: &Value) {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let _ = serde_json::to_writer_pretty(&mut stdout, value)
        .and_then(|_| stdout.write_all(b"\n").map_err(serde_json::Error::io));
}

pub fn event(value: &Value) -> bool {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).is_ok()
        && stdout.write_all(b"\n").is_ok()
        && stdout.flush().is_ok()
}
