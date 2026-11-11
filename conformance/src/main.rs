//! `conformance`: tooling for Wine's graphics conformance tests.
//!
//! `wine-macros` generates and checks the blocks in Wine's test sources that
//! let a single test function of a unit run alone ([`wine_src`]). Which
//! executables and units exist is in `wine-tests.toml` ([`model`]).

mod model;
mod wine_src;

use std::{path::PathBuf, process::ExitCode};

use clap::{ArgGroup, Args, Parser, Subcommand};

use crate::model::{Config, PeDir};

#[derive(Parser)]
#[command(
    name = "conformance",
    about = "Tooling for Wine's graphics conformance tests"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check or write the generated test selection blocks in a Wine source
    /// tree.
    WineMacros(WineMacros),
}

#[derive(Args)]
#[command(group(ArgGroup::new("mode").required(true).args(["check", "write"])))]
struct WineMacros {
    /// Fail when a block is missing or out of date, or when a call in a
    /// `START_TEST` body is neither wrapped nor setup.
    #[arg(long)]
    check: bool,
    /// Write or replace the blocks; a second run changes nothing.
    #[arg(long)]
    write: bool,
    /// The root of the Wine source tree.
    wine_src: PathBuf,
}

fn main() -> ExitCode {
    let Command::WineMacros(args) = Cli::parse().command;
    let config = match Config::builtin() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("conformance: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = if args.write {
        wine_src::write(&config, &args.wine_src).map(|written| {
            for file in &written {
                println!("wrote {file}");
            }
            println!("{} files changed", written.len());
        })
    } else {
        wine_src::check(&config, &args.wine_src).map(|units| {
            for unit in &units {
                let pe_dirs: Vec<&str> = config
                    .exes
                    .iter()
                    .filter(|exe| exe.name == unit.exe)
                    .flat_map(|exe| exe.pe_dirs.iter().map(PeDir::as_str))
                    .collect();
                let label = format!("{}/{} ({})", unit.exe, unit.unit, pe_dirs.join(", "));
                match &unit.keys {
                    Some(keys) => println!("{label}: {} keys", keys.len()),
                    None => println!("{label}: runs as a whole"),
                }
            }
            println!("{} units up to date", units.len());
        })
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(errors) => {
            for error in &errors {
                eprintln!("{error}");
            }
            eprintln!("conformance: {} problems", errors.len());
            ExitCode::FAILURE
        }
    }
}
