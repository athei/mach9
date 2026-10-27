mod bundle;
mod cli;
mod config;
mod dpi;
mod launch;
mod registry;
mod retina;
mod rules;
mod session;
#[cfg(test)]
mod test_keys;
mod ui;
mod update;
mod wine_env;
mod words;

use std::{convert::Infallible, env, fs, io, os::fd::RawFd, path::Path, process::ExitCode};

use compatdb_core::{
    DEFAULT_VALUE, Origin, Resolution, Scalar, ScalarValue, Setting, Specificity, VersionInfo,
};
use gamelauncher_core::games;
use log::{error, info};

use crate::{config::Config, dpi::Dpi, retina::RetinaMode, rules::LaunchRules};

fn main() -> ExitCode {
    let args = cli::parse();
    if let Some(wine) = args.log_pump {
        // The log pump the launcher starts on itself before it execs into
        // wine. First, ahead of everything that could touch AppKit or Launch
        // Services, and with no error dialog: it writes only to the log.
        launch::run_pump(nix::unistd::Pid::from_raw(wine));
        return ExitCode::SUCCESS;
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Logger may or may not be initialized when run() bails; write to
            // stderr unconditionally so the failure is always visible.
            let message = e.to_string();
            eprintln!("gamelauncher: {message}");
            // A Finder or Dock launch has no terminal to read that from.
            if bundle::launched_via_launch_services() {
                ui::error(&bundle::app_name(), &message);
            }
            ExitCode::FAILURE
        }
    }
}

fn run(args: cli::Args) -> io::Result<()> {
    if let Some(relaunch) = &args.relaunch {
        // The helper the launcher starts on itself after a dialog; it neither
        // logs nor touches the support directory.
        return bundle::relaunch_after(relaunch.pid, &relaunch.args);
    }
    let resources = bundle::resources_dir()?;
    let bundle_id = bundle::bundle_id(&resources)?;
    // Everything the launcher writes hangs off this one directory.
    let support = bundle::support_dir(&bundle_id)?;
    fs::create_dir_all(&support)?;

    // Before the config is read, so config failures reach the log too.
    let log_fd = maybe_redirect_stdio(&support)?;
    init_logger();

    match setup(&support, args)? {
        Some(cfg) => match launch(&cfg, &support, &resources, log_fd)? {},
        None => Ok(()),
    }
}

/// Everything up to the config of a launch that plays. `None` when this
/// process is done: it ran the Dock menu's Check for Updates, or it showed a
/// dialog or ran an update check and has started a fresh instance to play.
fn setup(support: &Path, mut args: cli::Args) -> io::Result<Option<Config>> {
    if args.check_updates {
        // The Dock menu's Check for Updates. Nothing is started either way:
        // the window is the whole point of the launch.
        update::check(true);
        return Ok(None);
    }

    if args.add_game || config::needs_first_game(support, &args.overrides)? {
        // The Dock menu's New Executable, and the first launch with nothing to
        // start. Like every dialog run, this one hands the launch to a fresh
        // instance, see below.
        if let Some(launch) = config::add_game(support)? {
            bundle::relaunch(&launch.args)?;
        }
        return Ok(None);
    }

    let cfg = Config::load_or_setup(support, args.overrides)?;
    if ui::used_gui() {
        // A dialog leaves this process registered with the window server for
        // good. Wine replaces this process, and running it in one that has
        // already checked in with the window server and Launch Services is
        // untested. So, as a precaution, a setup run ends here and a new one
        // plays.
        info!("setup finished; starting the game in a fresh instance");
        bundle::relaunch(&args.original)?;
        return Ok(None);
    }
    if args.skip_update_check {
        info!("update check skipped: resuming after the updater process exited");
    } else if update::check(false) == update::Outcome::RestartRequired {
        // Even a silent check can register this process with the window
        // server. Reuse the setup relaunch, and bypass Sparkle entirely in
        // the continuation, including after an error or a timeout.
        info!("update check finished; starting the game in a fresh instance");
        args.original.insert(0, "--skip-update-check".into());
        bundle::relaunch(&args.original)?;
        return Ok(None);
    }
    Ok(Some(cfg))
}

/// Prepare wine for `cfg` and start it. Returns only with an error: on
/// success wine replaces this process.
fn launch(
    cfg: &Config,
    support: &Path,
    resources: &Path,
    log_fd: Option<RawFd>,
) -> io::Result<Infallible> {
    let wine = resources.join("wine/bin/wine");
    let wineserver = resources.join("wine/bin/wineserver");
    let prefix = bundle::prefix_dir(support);
    fs::create_dir_all(&prefix)?;

    // First, while no other thread can be reading the environment, which
    // `set_var` needs: reading the display below already talks to the window
    // server. The only threads by now are the idle libdispatch workers an
    // update check's read of the user defaults may have started.
    wine_env::apply(&prefix, &cfg.rules.table);
    wine_env::apply_user(cfg);

    info!(
        "{} launcher starting (PID {})",
        cfg.display_name(),
        std::process::id()
    );
    let (retina, dpi) = display_settings(cfg.retina_mode, cfg.dpi);
    log_config(cfg, support, &prefix);

    let session = session::prepare(&prefix, support, &wineserver, retina, dpi)?;
    registry::ensure(
        &prefix,
        &wine,
        &wineserver,
        cfg.game_dir(),
        retina,
        dpi,
        session,
    );

    info!("execv into wine: {}", wine.display());
    log_fd.map_or_else(
        || launch::exec_direct(&wine, cfg.game_dir(), cfg.exe_name(), cfg.arguments()),
        |fd| launch::exec_via_pty(&wine, cfg.game_dir(), cfg.exe_name(), cfg.arguments(), fd),
    )
}

/// `on` or `off`, the way the log spells a switch.
#[must_use]
pub const fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// Retina mode and DPI for this launch, with `auto` resolved, logged the way
/// they were decided.
fn display_settings(retina_mode: RetinaMode, dpi: Dpi) -> (bool, u32) {
    let retina = retina_mode.resolve();
    match retina_mode {
        RetinaMode::Auto => info!(
            "RetinaMode: {} (auto, the main display is {})",
            on_off(retina),
            if retina { "HiDPI" } else { "not HiDPI" }
        ),
        RetinaMode::On | RetinaMode::Off => info!("RetinaMode: {}", on_off(retina)),
    }
    let resolved = dpi.resolve(retina);
    match dpi {
        Dpi::Auto => info!("DPI: {resolved} (auto, retina mode {})", on_off(retina)),
        Dpi::Fixed(_) => info!("DPI: {resolved} (fixed)"),
    }
    (retina, resolved)
}

/// The rest of what the launch runs with, for the log.
fn log_config(cfg: &Config, support: &Path, prefix: &Path) {
    if !cfg.env.is_empty() {
        let names: Vec<&str> = cfg.env.keys().map(String::as_str).collect();
        info!("Env overrides: {}", names.join(", "));
    }
    log_rule_settings(cfg);
    for line in list_settings(cfg) {
        info!("{line}");
    }
    info!("Process rules: {}", rule_lines(&cfg.rules));
    info!("Config: {}", bundle::config_path(support).display());
    match &cfg.game {
        Some(name) => info!(
            "Game: {name} ({})",
            games::game_path(support, name).display()
        ),
        None => info!("Game: none, --config executable in use"),
    }
    info!("WINEPREFIX: {}", prefix.display());
    info!("GameDir: {}", cfg.game_dir().display());
    info!("Executable: {}", cfg.executable().display());
    if !cfg.arguments().is_empty() {
        info!("Arguments: {:?}", cfg.arguments());
    }
}

/// The top-level settings that are lists, one line each with their entries,
/// for the log.
fn list_settings(cfg: &Config) -> Vec<String> {
    cfg.launch_wide
        .iter()
        .filter_map(|(key, value)| match value {
            rules::Value::List(entries) if !entries.is_empty() => {
                Some(format!("{}: {}", key.label(), entries.join(", ")))
            }
            _ => None,
        })
        .collect()
}

/// The settings of `compatdb.so`'s rules that hold one value, one line each,
/// for the log.
fn log_rule_settings(cfg: &Config) {
    for line in rule_settings(&cfg.rules) {
        info!("{line}");
    }
}

/// The lines [`log_rule_settings`] logs, one per setting of the rules that
/// holds one value, in the crate's order and with its name for a person.
/// Each says what a process that no rule names gets with this table, the
/// built-in rules included ([`compatdb_core::Table::preview`]), and which
/// rule that came from, then each rule that sets the setting for the
/// processes it matches. `compatdb.so`'s own log block for each process shows
/// what that process got, with the source of every value.
fn rule_settings(rules: &LaunchRules) -> Vec<String> {
    let every = rules.table.preview("", &VersionInfo::default());
    Scalar::ALL
        .iter()
        .map(|&scalar| {
            let label = scalar.key().label();
            format!("{label}: {}", summary(rules, &every, scalar))
        })
        .collect()
}

/// The line of one setting, see [`rule_settings`].
fn summary(rules: &LaunchRules, every: &Resolution, scalar: Scalar) -> String {
    let source = |origin: Option<&Origin>| origin.map(|origin| rules.describe(origin));
    let state = every.scalar(scalar);
    let mut summary = match (
        state.value,
        source(state.set_by.as_ref()),
        source(state.reset_by.as_ref()),
    ) {
        (Some(value), Some(from), _) => format!("{} ({from})", spelled(value)),
        (None, _, Some(from)) => {
            format!("{DEFAULT_VALUE} ({from}; upstream: {})", scalar.upstream())
        }
        _ => format!("not set (upstream: {})", scalar.upstream()),
    };
    // The rules that match every process with no fingerprint are in the
    // value above; every other rule names some processes.
    let by_rule: Vec<String> = rules
        .table
        .rules()
        .iter()
        .zip(&rules.names)
        .filter(|(rule, _)| rule.specificity() != Specificity::Any)
        .filter_map(|(rule, name)| {
            let value = match rule.scalar(scalar) {
                Setting::Set(value) => spelled(value),
                Setting::Reset => DEFAULT_VALUE,
                Setting::Unset => return None,
            };
            Some(format!("{}={value}", name.as_deref()?))
        })
        .collect();
    if !by_rule.is_empty() {
        summary.push_str("; rules: ");
        summary.push_str(&by_rule.join(", "));
    }
    summary
}

/// A value as the log spells it: a true-or-false one as `on` or `off`, any
/// other as the crate writes it.
const fn spelled(value: ScalarValue) -> &'static str {
    match value {
        ScalarValue::Bool(on) => on_off(on),
        value => value.as_str(),
    }
}

/// The table's rules by the line `compatdb.so`'s log names each by, so a
/// value the log gives by its line can be traced to its block.
fn rule_lines(rules: &LaunchRules) -> String {
    if rules.table.rules().is_empty() {
        return String::from("none");
    }
    let lines: Vec<String> = (1..=rules.table.rules().len())
        .map(|n| {
            let line = Origin::Line(n);
            format!("{} is the {}", line.label(), rules.describe(&line))
        })
        .collect();
    lines.join(", ")
}

/// Redirect stdout/stderr to the log file when the process was launched by
/// macOS `LaunchServices` (no terminal is attached). Returns the log fd so
/// the pty pump can write to it; `None` means a plain terminal launch.
fn maybe_redirect_stdio(support_dir: &Path) -> io::Result<Option<RawFd>> {
    if !bundle::launched_via_launch_services() {
        return Ok(None);
    }
    // SAFETY: launcher is single-threaded at this point.
    unsafe { env::set_var("NO_COLOR", "1") };
    let fd = bundle::redirect_stdio_to(&bundle::log_path(support_dir))?;
    Ok(Some(fd))
}

/// Info and up by default; `RUST_LOG` adds to that or replaces it.
fn init_logger() {
    let style = if env::var_os("NO_COLOR").is_some() {
        env_logger::WriteStyle::Never
    } else {
        env_logger::WriteStyle::Always
    };
    let result = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .write_style(style)
        .try_init();
    if result.is_err() {
        error!("logger already initialized");
    }
}

#[cfg(test)]
mod tests {
    use compatdb_core::{Dxgi, Exe, Key, OpenGl, Rule, Table, Wined3d};

    use super::*;
    use crate::test_keys::{flag, text};

    /// A `[process."<name>"]` block holding `lines`.
    fn block(name: &str, lines: &[String]) -> String {
        format!("[process.\"{name}\"]\n{}\n", lines.concat())
    }

    /// The [`Key::Exe`] line for `name`.
    fn exe(name: &str) -> String {
        text(Key::Exe, name)
    }

    #[test]
    fn auto_dpi_follows_the_resolved_retina_mode() {
        assert_eq!(display_settings(RetinaMode::On, Dpi::Auto), (true, 192));
        assert_eq!(display_settings(RetinaMode::Off, Dpi::Auto), (false, 96));
        assert_eq!(
            display_settings(RetinaMode::On, Dpi::Fixed(144)),
            (true, 144)
        );
        assert_eq!(
            display_settings(RetinaMode::Off, Dpi::Fixed(120)),
            (false, 120)
        );
        // Whatever display the machine running the tests has.
        let (retina, dpi) = display_settings(RetinaMode::Auto, Dpi::Auto);
        assert_eq!(dpi, if retina { 192 } else { 96 });
    }

    fn launch_rules(own: Rule, toml: &str) -> LaunchRules {
        let doc: toml_edit::DocumentMut = toml.parse().unwrap();
        let named = doc
            .get("process")
            .map(|item| {
                rules::parse(item, &mut rules::Ignored::default())
                    .expect("valid")
                    .rules
            })
            .unwrap_or_default();
        rules::table(own, named)
    }

    fn every() -> Rule {
        Rule::new(Exe::Any)
    }

    /// The value of the line of `scalar` among `lines`.
    fn line_of(lines: &[String], scalar: Scalar) -> String {
        let label = scalar.key().label();
        lines
            .iter()
            .find_map(|line| line.strip_prefix(&format!("{label}: ")))
            .unwrap_or_else(|| panic!("no {label} line in {lines:?}"))
            .to_owned()
    }

    /// The line of `scalar` when the launcher sends no rules: what the
    /// built-in rules give a process no rule names.
    fn built_in_line(scalar: Scalar) -> String {
        line_of(&rule_settings(&LaunchRules::default()), scalar)
    }

    #[test]
    fn the_line_names_the_value_its_block_and_the_rules_that_set_it() {
        let dpi_aware = |own: &Rule, toml: &str| {
            line_of(
                &rule_settings(&launch_rules(own.clone(), toml)),
                Scalar::DpiAware,
            )
        };
        let unset = every();
        let on = every().with_dpi_aware(true);
        assert_eq!(dpi_aware(&unset, ""), built_in_line(Scalar::DpiAware));
        assert_eq!(dpi_aware(&on, ""), "on (top level)");
        let rules = [
            block("game", &[exe("game.exe"), flag(Key::DpiAware, false)]),
            block("plain", &[exe("plain.exe")]),
            block(
                "tool",
                &[
                    exe(Exe::ANY),
                    text(Key::Company, "Acme"),
                    flag(Key::DpiAware, true),
                ],
            ),
        ]
        .concat();
        assert_eq!(
            dpi_aware(&on, &rules),
            "on (top level); rules: game=off, tool=on"
        );
        assert_eq!(
            dpi_aware(&unset, &rules),
            format!(
                "{}; rules: game=off, tool=on",
                built_in_line(Scalar::DpiAware)
            )
        );
        // A rule for every process folds after the top level and names its
        // own block.
        assert_eq!(
            dpi_aware(
                &on,
                &block("all", &[exe(Exe::ANY), flag(Key::DpiAware, false)])
            ),
            "off (rule `all`)"
        );
    }

    /// Without a rule of the launcher, a line gives what the built-in rules
    /// set, or what upstream Wine does where none sets the setting.
    #[test]
    fn a_line_the_launcher_sets_nothing_for_says_where_the_value_comes_from() {
        let every = Table::new().preview("", &VersionInfo::default());
        for &scalar in Scalar::ALL {
            let state = every.scalar(scalar);
            let expected = match (state.value, &state.set_by) {
                (Some(value), Some(origin)) => format!("{} ({})", spelled(value), origin.label()),
                _ => format!("not set (upstream: {})", scalar.upstream()),
            };
            assert_eq!(built_in_line(scalar), expected, "{scalar}");
        }
    }

    /// `default` hands a setting back to upstream Wine; the line says what
    /// that is, and a rule that resets it reads `default`.
    #[test]
    fn a_reset_reads_default_with_what_upstream_wine_does() {
        let global = every()
            .with_dpi_aware(Setting::Reset)
            .with_dxgi(Setting::Reset);
        let lines = rule_settings(&launch_rules(
            global,
            &block(
                "game",
                &[
                    exe("Game.exe"),
                    text(Key::Company, "Acme"),
                    text(Key::Dxgi, Dxgi::Wined3d),
                    text(Key::D3d9, DEFAULT_VALUE),
                ],
            ),
        ));
        assert_eq!(
            line_of(&lines, Scalar::DpiAware),
            format!(
                "default (top level; upstream: {})",
                Scalar::DpiAware.upstream()
            )
        );
        assert_eq!(
            line_of(&lines, Scalar::Dxgi),
            format!(
                "default (top level; upstream: {}); rules: game={}",
                Scalar::Dxgi.upstream(),
                Dxgi::Wined3d
            )
        );
        assert_eq!(
            line_of(&lines, Scalar::D3d9),
            format!(
                "{}; rules: game={DEFAULT_VALUE}",
                built_in_line(Scalar::D3d9)
            )
        );
    }

    #[test]
    fn each_line_reads_its_own_setting() {
        let global = every()
            .with_force_nx(true)
            .with_wined3d(Wined3d::Vulkan)
            .with_opengl(OpenGl::Cgl);
        let lines = rule_settings(&launch_rules(
            global,
            &[
                block(
                    "old-game",
                    &[exe("OldGame.exe"), flag(Key::LargeAddressAware, true)],
                ),
                block("tuned", &[exe("Tuned.exe"), flag(Key::Gcmouse, false)]),
                block("setup", &[exe("Setup.exe"), flag(Key::ForceNx, false)]),
                block(
                    "tool",
                    &[
                        exe("Tool.exe"),
                        text(Key::Dxgi, Dxgi::Wined3d),
                        text(Key::OpenGl, OpenGl::Egl),
                    ],
                ),
            ]
            .concat(),
        ));
        let labels: Vec<&str> = lines
            .iter()
            .filter_map(|line| line.split_once(": ").map(|(label, _)| label))
            .collect();
        let expected: Vec<&str> = Scalar::ALL.iter().map(|s| s.key().label()).collect();
        assert_eq!(labels, expected);
        let built_in = |scalar, rules: &str| format!("{}; rules: {rules}", built_in_line(scalar));
        assert_eq!(
            line_of(&lines, Scalar::LargeAddressAware),
            built_in(Scalar::LargeAddressAware, "old-game=on")
        );
        assert_eq!(
            line_of(&lines, Scalar::Gcmouse),
            built_in(Scalar::Gcmouse, "tuned=off")
        );
        assert_eq!(
            line_of(&lines, Scalar::ForceNx),
            "on (top level); rules: setup=off"
        );
        assert_eq!(
            line_of(&lines, Scalar::Dxgi),
            built_in(Scalar::Dxgi, &format!("tool={}", Dxgi::Wined3d))
        );
        assert_eq!(line_of(&lines, Scalar::D3d9), built_in_line(Scalar::D3d9));
        assert_eq!(
            line_of(&lines, Scalar::Wined3d),
            format!("{} (top level)", Wined3d::Vulkan)
        );
        assert_eq!(
            line_of(&lines, Scalar::OpenGl),
            format!("{} (top level); rules: tool={}", OpenGl::Cgl, OpenGl::Egl)
        );
    }

    /// compatdb.so's log names each value of the table by its line; the
    /// launcher's log says which block each line is.
    #[test]
    fn the_rules_line_maps_each_rule_to_its_block() {
        assert_eq!(rule_lines(&launch_rules(every(), "")), "none");
        let global = every().with_gcmouse(false);
        let line = |n| Origin::Line(n).label();
        let tool = block("tool", &[exe("Tool.exe"), text(Key::Company, "Acme")]);
        assert_eq!(
            rule_lines(&launch_rules(global, &tool)),
            format!(
                "{} is the top level, {} is the rule `tool`",
                line(1),
                line(2)
            )
        );
        assert_eq!(
            rule_lines(&launch_rules(every(), &tool)),
            format!("{} is the rule `tool`", line(1))
        );
    }
}
