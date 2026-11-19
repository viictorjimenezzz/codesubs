mod args;
mod commands;
mod output;
mod shell;

use args::Args;
use clap::Parser;

#[tokio::main]
async fn main() {
    let raw = std::env::args_os().collect::<Vec<_>>();
    let requested = |flag: &str| {
        raw.iter()
            .take_while(|argument| argument.as_os_str() != "--")
            .any(|argument| argument == flag)
    };
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            if error.use_stderr() && (requested("--json") || requested("--json-events")) {
                output::errors::report(
                    "invalid_input",
                    &error.to_string(),
                    true,
                    requested("--json-events"),
                );
                std::process::exit(2);
            }
            error.exit();
        }
    };
    let machine = args.json;
    let events = args.json_events;
    if let Err(error) = commands::run(args).await {
        output::errors::report(&error.code, &error.message, machine, events);
        std::process::exit(output::errors::exit_code(&error.code));
    }
}
