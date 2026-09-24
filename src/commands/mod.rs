//! Command router and submodules.

pub mod account;
pub mod attachment;
pub mod doctor;
pub mod draft;
pub mod label;
pub mod message;
pub mod search;
pub mod setup;
pub mod thread;

use crate::cli::{Cli, Command};
use crate::error::Result;
use crate::output;

pub fn execute(cli: Cli) -> Result<()> {
    let verbose = cli.verbose;
    let timeout = cli.timeout.unwrap_or(100);

    match cli.command {
        Command::Setup(args) => setup::run(args)?,
        Command::Login(args) => account::run(crate::cli::AccountCommand::Add(args))?,
        Command::Account { command } => account::run(command)?,

        Command::Search(args) => {
            let val = search::run(args, verbose, timeout)?;
            output::write(&val);
        }
        Command::Message { command } => {
            let val = message::run(command, verbose, timeout)?;
            output::write(&val);
        }
        Command::Thread { command } => {
            let val = thread::run(command, verbose, timeout)?;
            output::write(&val);
        }
        Command::Attachment { command } => {
            let val = attachment::run(command, verbose, timeout)?;
            output::write(&val);
        }
        Command::Label { command } => {
            let val = label::run(command, verbose, timeout)?;
            output::write(&val);
        }
        Command::Draft { command } => {
            let val = draft::run(command, verbose, timeout)?;
            output::write(&val);
        }
        Command::Doctor(args) => {
            let val = doctor::run(args, verbose, timeout)?;
            output::write(&val);
        }
        Command::AgentReadme(args) => {
            crate::readme::run(args.format.as_deref());
        }
    }

    Ok(())
}
