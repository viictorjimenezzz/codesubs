use codesubs_core::AccountRecord;

pub fn accounts(accounts: &[AccountRecord]) {
    if accounts.is_empty() {
        println!("No accounts yet. Run codesubs add <codex|claude> --email <email>.");
        return;
    }
    println!(
        "{:<32} {:<30} {:<12} {:>10} {:>12}",
        "ACCOUNT", "EMAIL", "PLAN", "WEEK LEFT", "SESSION LEFT"
    );
    for a in accounts {
        let weekly = a
            .quota
            .as_ref()
            .and_then(|q| q.weekly())
            .map(|w| format!("{}%", w.remaining_percent_at(chrono::Utc::now())))
            .unwrap_or_else(|| "–".into());
        let session = a
            .quota
            .as_ref()
            .and_then(|q| q.session())
            .map(|w| format!("{}%", w.remaining_percent_at(chrono::Utc::now())))
            .unwrap_or_else(|| "–".into());
        println!(
            "{:<32} {:<30} {:<12} {:>10} {:>12}",
            a.reference(),
            a.email.as_deref().unwrap_or("–"),
            a.plan.as_deref().unwrap_or("–"),
            weekly,
            session
        );
    }
}
