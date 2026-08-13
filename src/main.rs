//! The `diurn` command.
//!
//! MIC subcommands work entirely offline against a built-in snapshot or a file
//! you point them at. `mic fetch` is the sole exception and the only place a
//! network dependency is linked in at all.

mod cli;
mod commands;
mod fetch;
mod output;
mod render;
mod source;

use clap::{error::ErrorKind, Parser};

use cli::{Cli, Command, Format};
use commands::{Ctx, EXIT_OK, EXIT_USAGE};

fn main() {
    // Parsed by hand rather than via `Cli::parse()` so that the exit codes are
    // ours: clap exits 2 on a usage error, which SPEC Phase 2 assigns to "load
    // produced errors". Help and version are not failures and exit 0.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let code = match e.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => EXIT_OK,
                _ => EXIT_USAGE,
            };
            let _ = e.print();
            std::process::exit(code);
        }
    };

    let ctx = Ctx {
        format: Format::resolve(cli.format),
        quiet: cli.quiet,
    };

    let result = match cli.command {
        Command::Mic(cmd) => commands::run(cmd, &ctx),
        Command::Cal(_) => commands::cal(ctx.quiet),
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            // A closed pipe is what `| head` looks like from in here. Doing
            // exactly what was asked is not an error.
            if let Some(io) = e.downcast_ref::<std::io::Error>() {
                if io.kind() == std::io::ErrorKind::BrokenPipe {
                    std::process::exit(EXIT_OK);
                }
            }
            eprintln!("error: {e:#}");
            std::process::exit(EXIT_USAGE);
        }
    }
}
