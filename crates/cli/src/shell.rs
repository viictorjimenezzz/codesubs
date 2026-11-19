use std::collections::BTreeMap;

pub fn environment(set: &BTreeMap<String, String>, unset: &[String], fish: bool) -> String {
    let mut lines = set
        .iter()
        .map(|(key, value)| {
            let escaped = if fish {
                value.replace('\\', "\\\\").replace('\'', "\\'")
            } else {
                value.replace('\'', "'\\''")
            };
            let quoted = format!("'{escaped}'");
            if fish {
                format!("set -gx {key} {quoted}")
            } else {
                format!("export {key}={quoted}")
            }
        })
        .collect::<Vec<_>>();
    if !unset.is_empty() {
        lines.push(format!(
            "{} {}",
            if fish { "set -e" } else { "unset" },
            unset.join(" ")
        ));
    }
    lines.join("\n")
}
