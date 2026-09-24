//! `gmail` - A CLI over one or more Gmail mailboxes, built to be driven by an LLM agent.

mod account;
mod auth;
mod cli;
mod client;
mod commands;
mod config;
mod error;
mod mail;
mod output;
mod readme;
mod secrets;
mod util;

use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;

use crate::error::Error;

fn main() -> ExitCode {
    // The error envelope can be rendered before parsing succeeds, so the format has to be known
    // from the raw args first.
    let json_prescan =
        std::env::args_os().skip(1).take_while(|a| a != "--").any(|a| a == "--json" || a == "--format=json");
    output::set_json(json_prescan);

    let cli = match cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return parse_error(e),
    };

    let is_json = cli.json || cli.format.as_deref().map(|f| f.eq_ignore_ascii_case("json")).unwrap_or(false);
    output::set_json(is_json);

    match commands::execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => ExitCode::from(output::write_error(&e) as u8),
    }
}

/// Help and version are successes; everything else clap refuses is an `invalid_input` envelope,
/// with clap's own explanation (and usage) as the detail.
fn parse_error(e: clap::Error) -> ExitCode {
    match e.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let _ = e.print();
            ExitCode::SUCCESS
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let _ = e.print();
            ExitCode::from(error::ErrorCode::InvalidInput as u8)
        }
        _ => {
            let rendered = e.render().to_string();
            let (what, usage) = rendered.split_once("\n\n").unwrap_or((&rendered, ""));
            let message = what
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
                .trim_start_matches("error: ")
                .to_string();
            let mut err = Error::invalid(if message.is_empty() { "Invalid arguments.".into() } else { message });
            if !usage.trim().is_empty() {
                err = err.detail(usage.trim());
            }
            ExitCode::from(output::write_error(&err) as u8)
        }
    }
}
