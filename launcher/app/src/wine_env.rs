use std::{env, ffi::OsString, path::Path};

use compatdb_core::{ENV_VAR as COMPATDB_ENV, Table, inherited_conflicts};

use crate::config::Config;

/// Debug channels the launcher always wants off.
///
/// `fixme-all` goes first (class syntax, a bare `-fixme` would name a
/// channel): every stub message from every channel, thousands per session
/// (`setsockopt`, `NtQueryInformationProcess`,
/// `dxgi_surface_GetDC`), none of which has ever named a problem; `err` and
/// `warn` stay on. The dbghelp debug-info parsers are silenced as whole
/// channels because they also spam while winedbg produces a crash backtrace;
/// the general dbghelp channel stays on, module-loading errors are worth
/// seeing. Msync needs no channel here: the server banner is an unconditional
/// fprintf and client-side msync failures are ERRs on the default-enabled sync
/// channel. `WINEDEBUG` inherited from the environment is ignored: the rules'
/// `WINEDEBUG` entries ([`Table::winedebug`]) are the only way to add
/// channels, and a `+fixme` entry turns the stub messages back on.
const WINEDEBUG_CHANNELS: &str =
    "fixme-all,-dbghelp_msc,-dbghelp_dwarf,-dbghelp_pdb,-dbghelp_macho";

/// Remove the inherited variables that would make the launch depend on where
/// the launcher was started from, and set or remove the ones the launcher
/// owns outright, see [`launcher_env`].
///
/// compatdb-core names the inherited variables ([`inherited_conflicts`]): a
/// relaunch through `open -n` drops them, and a Finder or Dock launch never
/// has them. A config may still set one of them in `[env]`, which
/// [`apply_user`] applies, unless a setting of the rules replaced it.
pub fn apply(prefix: &Path, rules: &Table) {
    for name in inherited_conflicts() {
        // SAFETY: launcher is single-threaded at this point.
        unsafe { env::remove_var(name) };
    }
    for (name, value) in launcher_env(prefix, rules) {
        match value {
            // SAFETY: as above.
            Some(value) => unsafe { env::set_var(name, value) },
            // SAFETY: as above.
            None => unsafe { env::remove_var(name) },
        }
    }
}

/// The variables the launcher owns outright, each with its value, or `None`
/// for one the launch must not have, which is removed even when the launcher
/// inherited it. None of these is a setting. `WINEPREFIX` and `WINEMSYNC` are
/// prefix-wide, and [`COMPATDB_ENV`] carries the config's rules, written by
/// the table itself. A config without rules has an empty table, and the
/// variable is left out rather than set empty, so `compatdb.so` applies its
/// built-in rules alone, as it does for a process tree the launcher did not
/// start.
fn launcher_env(prefix: &Path, rules: &Table) -> [(&'static str, Option<OsString>); 3] {
    let compatdb = (!rules.rules().is_empty()).then(|| OsString::from(rules.to_string()));
    [
        ("WINEPREFIX", Some(prefix.as_os_str().to_owned())),
        ("WINEMSYNC", Some(OsString::from("1"))),
        (COMPATDB_ENV, compatdb),
    ]
}

/// Apply the settings the config controls. Runs after [`apply`], so a setting
/// wins over the launcher's own value for the same variable.
///
/// `WINEDEBUG` is the launcher's own channels followed by the rules' entries;
/// `[env]` is passed through as written, and the config has refused a
/// variable there that a setting of the rules replaced. Every other variable
/// a setting of the rules stands for is set by `compatdb.so` in each process
/// ([`DocSection::Folding`](compatdb_core::DocSection::Folding)).
pub fn apply_user(cfg: &Config) {
    // SAFETY: launcher is single-threaded at this point.
    unsafe {
        env::set_var("WINEDEBUG", winedebug(&cfg.rules.table));
        for (name, value) in &cfg.env {
            env::set_var(name, value);
        }
    }
}

/// Compose `WINEDEBUG`: the launcher's channels, then the rules' entries
/// ([`Table::winedebug`]). Wine reads the variable
/// before `compatdb.so` loads, so the launcher has to put them there.
///
/// Wine reads the list left to right and lets a later entry win for the same
/// channel, so a config only has to name what it wants to change, and
/// `+dbghelp_pdb` turns back on a channel the launcher turns off.
fn winedebug(rules: &Table) -> String {
    let mut parts = vec![String::from(WINEDEBUG_CHANNELS)];
    parts.extend(rules.winedebug());
    parts.join(",")
}

#[cfg(test)]
mod tests;
