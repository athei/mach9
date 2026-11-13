//! The `conformance` command line, on top of the library of the same name.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::Ordering,
    time::Duration,
};

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use conformance::{
    activation::Host,
    baseline,
    deqp::{self, groups::Groups},
    exec::Limits,
    interrupt, layout,
    model::{Config, PeDir},
    resume::Mode,
    runner::{self, Options},
    session::Failure,
    wine_src,
};

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
    /// Run the tests of the suites and compare them with their committed
    /// baselines.
    ///
    /// Exits with 0 when every test matches, 1 for a regression, a stale
    /// baseline or unstable keys (the last line says which), and 2 for a
    /// usage or setup error.
    Run(RunArgs),
    /// Run the tests of the suites, again where a result changed, and write
    /// their baselines.
    ///
    /// Exits with 0 when the baseline is stable, 1 when keys were unstable,
    /// and 2 for a usage or setup error.
    Record(RunArgs),
    /// Run the groups of `deqp-vk.toml` on KosmicKrisp with `deqp-vk`, or
    /// record their expected results.
    #[command(subcommand)]
    DeqpVk(DeqpCommand),
}

#[derive(Subcommand)]
enum DeqpCommand {
    /// Run the cases of the groups and compare them with their committed
    /// expected results.
    ///
    /// Exits with 0 when every case matches, 1 for a regression, stale
    /// expected results or unstable cases (the last line says which), and 2
    /// for a usage or setup error.
    Run(DeqpArgs),
    /// Run the cases of the groups, again where a result changed, and write
    /// their expected results.
    ///
    /// Exits with 0 when the results are stable, 1 when cases were
    /// unstable, and 2 for a usage or setup error.
    Record(DeqpArgs),
}

#[derive(Args)]
struct DeqpArgs {
    /// Run this group of deqp-vk.toml; may be given several times. By
    /// default every group.
    #[arg(long = "group", value_name = "NAME")]
    groups: Vec<String>,
    /// `run` only: run the cases of the groups this pattern matches (`*`
    /// matches any run of characters), and compare only those; may be given
    /// several times.
    #[arg(long = "cases", value_name = "PATTERN")]
    cases: Vec<String>,
    /// The Vulkan stack: a full bundle or a `MESA_DIST` of
    /// bin/build-mesa.sh; by default `<root>/dist/wine`, where `<root>` is
    /// `WINE_BUILD_ROOT` or the directory that holds this checkout.
    #[arg(long, value_name = "DIR")]
    vulkan: Option<PathBuf>,
    /// The deqp-vk that bin/build-vk-gl-cts.sh staged; by default
    /// `<root>/build/vk-gl-cts/dist`.
    #[arg(long, value_name = "DIR")]
    deqp: Option<PathBuf>,
    /// The runner's raw logs; by default `<root>/build/deqp-vk`.
    #[arg(long, value_name = "DIR")]
    state_dir: Option<PathBuf>,
    /// The expected results and their triage file; by default the
    /// committed `conformance/baselines/deqp-vk`.
    #[arg(long, value_name = "DIR")]
    expected: Option<PathBuf>,
    /// How many deqp-vk processes run at a time.
    #[arg(long, value_name = "N", default_value_t = DEQP_JOBS,
          value_parser = clap::value_parser!(u64).range(1..))]
    jobs: u64,
    /// How many cases one deqp-vk process runs at most.
    #[arg(long, value_name = "N", default_value_t = 500,
          value_parser = clap::value_parser!(u64).range(1..))]
    batch: u64,
    /// A case that runs this long is a stall: a case prints a line when it
    /// starts and when it ends.
    #[arg(long, value_name = "SECONDS", default_value_t = 60,
          value_parser = clap::value_parser!(u64).range(1..))]
    idle_limit: u64,
    /// A deqp-vk process that runs longer than this is a stall.
    #[arg(long, value_name = "SECONDS", default_value_t = 3600,
          value_parser = clap::value_parser!(u64).range(1..))]
    time_limit: u64,
}

/// The default of `deqp-vk --jobs`.
const DEQP_JOBS: u64 = 8;

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

#[derive(Args)]
struct RunArgs {
    /// Test executables as wine-tests.toml names them (d3d11, dxgi, ddraw,
    /// vulkan-1, ...), or `all`.
    #[arg(required = true, value_name = "SUITE")]
    suites: Vec<String>,
    /// Run this leg only; by default every leg the executable is built for.
    #[arg(long, value_enum)]
    leg: Option<Leg>,
    /// Run these keys only, of exactly one suite; `<unit>/*` for a unit
    /// that runs as a whole.
    #[arg(long, value_name = "UNIT/KEY,...")]
    only: Option<String>,
    /// The bundle; by default `<root>/dist/wine`, where `<root>` is
    /// `WINE_BUILD_ROOT` or the directory that holds this checkout.
    #[arg(long, value_name = "DIR")]
    bundle: Option<PathBuf>,
    /// The runner's prefix, raw logs and resume state; by default
    /// `<root>/build/conformance`.
    #[arg(long, value_name = "DIR")]
    state_dir: Option<PathBuf>,
    /// The baselines and the triage file; by default the committed
    /// `conformance/baselines`.
    #[arg(long, value_name = "DIR")]
    baselines: Option<PathBuf>,
    /// A test that prints nothing for this long is a stall.
    #[arg(long, value_name = "SECONDS", default_value_t = 45,
          value_parser = clap::value_parser!(u64).range(1..))]
    idle_limit: u64,
    /// A test that runs longer than this is a stall.
    #[arg(long, value_name = "SECONDS", default_value_t = 900,
          value_parser = clap::value_parser!(u64).range(1..))]
    time_limit: u64,
    /// `run` only: the WINEDEBUG of every Wine process, such as
    /// `+win,+event`, to trace a test. The traces go into the raw logs.
    #[arg(long, value_name = "CHANNELS")]
    winedebug: Option<String>,
}

/// A leg as the command line spells it.
#[derive(Clone, Copy, ValueEnum)]
enum Leg {
    #[value(name = "i386")]
    I386,
    #[value(name = "x86_64")]
    X86_64,
}

impl From<Leg> for PeDir {
    fn from(leg: Leg) -> Self {
        match leg {
            Leg::I386 => Self::I386,
            Leg::X86_64 => Self::X86_64,
        }
    }
}

fn main() -> ExitCode {
    if let Err(e) = interrupt::install() {
        eprintln!("conformance: {e}");
        return ExitCode::from(2);
    }
    let command = Cli::parse().command;
    let config = match Config::builtin() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("conformance: {e}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::WineMacros(args) => wine_macros(&config, &args),
        Command::Run(args) => run(&config, Mode::Run, args),
        Command::Record(args) => run(&config, Mode::Record, args),
        Command::DeqpVk(DeqpCommand::Run(args)) => deqp_vk(Mode::Run, args),
        Command::DeqpVk(DeqpCommand::Record(args)) => deqp_vk(Mode::Record, args),
    }
}

/// `deqp-vk run` and `deqp-vk record`.
fn deqp_vk(mode: Mode, args: DeqpArgs) -> ExitCode {
    let groups = match Groups::builtin() {
        Ok(groups) => groups,
        Err(e) => {
            eprintln!("conformance: {e}");
            return ExitCode::from(2);
        }
    };
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = layout::root(std::env::var_os("WINE_BUILD_ROOT").as_deref(), manifest);
    let count = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
    let options = deqp::run::Options {
        mode,
        groups: args.groups,
        cases: args.cases,
        vulkan: args.vulkan.unwrap_or_else(|| layout::default_bundle(&root)),
        deqp: args.deqp.unwrap_or_else(|| layout::default_deqp(&root)),
        state: args
            .state_dir
            .unwrap_or_else(|| layout::default_deqp_state(&root)),
        expected: args.expected.unwrap_or_else(deqp::expected::dir),
        jobs: count(args.jobs),
        batch: count(args.batch),
        limits: Limits {
            idle: Duration::from_secs(args.idle_limit),
            total: Duration::from_secs(args.time_limit),
        },
    };
    finish(deqp::run::execute(
        &groups,
        &options,
        &std::env::vars_os().collect::<Vec<_>>(),
        &interrupt::REQUESTED,
    ))
}

/// Print how a command ended and give its exit code.
fn finish(result: Result<runner::Conclusion, Failure>) -> ExitCode {
    match result {
        Ok(conclusion) => {
            println!("{}", conclusion.last_line());
            ExitCode::from(conclusion.exit_code())
        }
        Err(failure) => {
            // A step the interrupt killed fails as a setup error.
            let failure = if interrupt::REQUESTED.load(Ordering::SeqCst) {
                Failure::Interrupted
            } else {
                failure
            };
            eprintln!("conformance: {failure}");
            ExitCode::from(failure.exit_code())
        }
    }
}

/// `run` and `record`.
fn run(config: &Config, mode: Mode, args: RunArgs) -> ExitCode {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = layout::root(std::env::var_os("WINE_BUILD_ROOT").as_deref(), manifest);
    let options = Options {
        mode,
        suites: args.suites,
        leg: args.leg.map(PeDir::from),
        only: args.only,
        bundle: args.bundle.unwrap_or_else(|| layout::default_bundle(&root)),
        state: args
            .state_dir
            .unwrap_or_else(|| layout::default_state(&root)),
        baselines: args.baselines.unwrap_or_else(baseline::dir),
        limits: Limits {
            idle: Duration::from_secs(args.idle_limit),
            total: Duration::from_secs(args.time_limit),
        },
        winedebug: args.winedebug,
    };
    // Read before the runner becomes an application, which may set
    // variables of its own.
    let caller_env: Vec<_> = std::env::vars_os().collect();
    // Everything that needs no session is checked before the runner takes
    // the activation, so a run that cannot start takes none, and none from
    // a run already in progress.
    let started = runner::prepare(config, &options).and_then(|prepared| {
        let host = Host::start(
            &prepared.bundle.dir,
            &prepared.state.prefix(),
            &interrupt::REQUESTED,
        )?;
        Ok((prepared, host))
    });
    let (prepared, host) = match started {
        Ok(started) => started,
        Err(failure) => return finish(Err(failure)),
    };
    finish(host.serve(move || prepared.run(&options, caller_env, &interrupt::REQUESTED)))
}

/// `wine-macros`.
fn wine_macros(config: &Config, args: &WineMacros) -> ExitCode {
    let result = if args.write {
        wine_src::write(config, &args.wine_src).map(|written| {
            for file in &written {
                println!("wrote {file}");
            }
            println!("{} files changed", written.len());
        })
    } else {
        wine_src::check(config, &args.wine_src).map(|units| {
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
