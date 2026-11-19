use serde_json::json;
use std::io::Write;

pub fn exit_code(code: &str) -> i32 {
    match code {
        "invalid_input" | "invalid_slug" | "account_not_found" | "ambiguous_reference" => 2,
        "no_available_account" => 3,
        "executable_not_found" => 127,
        "cancelled" => 130,
        _ => 1,
    }
}

pub fn report(code: &str, message: &str, machine: bool, events: bool) {
    if machine || events {
        let _ = writeln!(
            std::io::stderr().lock(),
            "{}",
            json!({"protocolVersion":1,"error":{"code":code,"message":message}})
        );
        if events {
            super::json::event(
                &json!({"protocolVersion":1,"type":"error","event":"error","code":code,"message":message}),
            );
        }
    } else {
        let _ = writeln!(std::io::stderr().lock(), "codesubs: {message}");
    }
}
