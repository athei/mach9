//! The four registry values the launcher keeps in the prefix: the Mac driver's
//! retina mode and the system DPI (`LogPixels`), which follow the config, the
//! `WineDbg` crash dialog, which stays off, and win32u's `EmulateModeset`,
//! which stays on.
//!
//! win32u reads `LogPixels` from `HKCU\Control Panel\Desktop` once per
//! process, falling back to the 96 of `HKCC\Software\Fonts` when the value is
//! missing, and the desktop process gives each monitor its own DPI when the
//! session enumerates the displays. So, like retina mode, the value has to be
//! in place before the session boots (`dpi.rs` says how it is chosen).
//!
//! `EmulateModeset` is what keeps a game's display-mode change virtual. A
//! Direct3D implementation sets the mode a fullscreen game picks through
//! `ChangeDisplaySettings`, as on Windows; with the key on, win32u keeps the
//! physical display as it is, scales the game window onto it and maps mouse
//! input into the mode, so the game's clicks land where its UI is drawn.
//! Without it the Mac driver hands the request to `CGDisplaySetDisplayMode`
//! and the whole desktop switches resolution. win32u reads the key under the
//! `X11 Driver` key whatever the driver is, once per process at start, and a
//! wineserver session computes its mode list when its desktop starts, so the
//! key has to be in place before the session that runs the game boots.
//!
//! `wine reg add` boots the whole prefix (wineserver, services, explorer) for a
//! one-line change and costs several seconds, so when the values are already
//! right nothing runs, and when the launch boots a fresh session the launcher
//! writes `user.reg` itself: that file is what the server loads at start. It
//! does so holding the lock a wineserver takes for its lifetime (see
//! `session.rs`), so a server cannot start and load the old file halfway
//! through. `reg add` is the fallback when a session is running, which then
//! already booted with the wanted retina mode and DPI (`session::prepare`
//! restarts one that did not), when it is cheap anyway (no boot, just a
//! process), or when the prefix has not been created yet.

use std::{fs, path::Path, process::Command};

use gamelauncher_core::fsutil::write_atomic;
use log::{info, warn};

use crate::session::{self, Session};

/// A value to hold in `HKCU`: the key (in `user.reg`'s doubled-backslash
/// spelling), the value name, and the value as `reg add` takes it and as
/// `user.reg` spells it.
#[derive(Debug, PartialEq, Eq)]
struct Setting {
    key: &'static str,
    name: &'static str,
    kind: &'static str,
    value: String,
    reg_text: String,
}

impl Setting {
    fn retina_mode(enabled: bool) -> Self {
        let value = if enabled { "Y" } else { "N" };
        Self {
            key: r"Software\\Wine\\Mac Driver",
            name: "RetinaMode",
            kind: "REG_SZ",
            value: value.to_owned(),
            reg_text: format!("\"{value}\""),
        }
    }

    fn log_pixels(dpi: u32) -> Self {
        Self {
            key: r"Control Panel\\Desktop",
            name: "LogPixels",
            kind: "REG_DWORD",
            value: dpi.to_string(),
            reg_text: format!("dword:{dpi:08x}"),
        }
    }

    fn crash_dialog_off() -> Self {
        Self {
            key: r"Software\\Wine\\WineDbg",
            name: "ShowCrashDialog",
            kind: "REG_DWORD",
            value: String::from("0"),
            reg_text: String::from("dword:00000000"),
        }
    }

    fn emulate_modeset() -> Self {
        Self {
            key: r"Software\\Wine\\X11 Driver",
            name: "EmulateModeset",
            kind: "REG_SZ",
            value: String::from("Y"),
            reg_text: String::from("\"Y\""),
        }
    }

    fn line(&self) -> String {
        format!("\"{}\"={}", self.name, self.reg_text)
    }

    /// The `reg add` key: `HKCU\` plus the key with single backslashes.
    fn hkcu_key(&self) -> String {
        format!("HKCU\\{}", self.key.replace("\\\\", "\\"))
    }
}

fn settings(retina_enabled: bool, dpi: u32) -> [Setting; 4] {
    [
        Setting::retina_mode(retina_enabled),
        Setting::log_pixels(dpi),
        Setting::crash_dialog_off(),
        Setting::emulate_modeset(),
    ]
}

/// How the values get into the registry.
#[derive(Debug, PartialEq, Eq)]
enum Plan<'a> {
    /// `user.reg` already holds every one.
    Nothing,
    /// A fresh session is next and its lock is held: write `user.reg` as
    /// `text`, which adds the `missing` values.
    WriteFile {
        text: String,
        missing: Vec<&'a Setting>,
    },
    /// A session is up, or the prefix has no `user.reg` yet: `reg add` each.
    RegAdd(Vec<&'a Setting>),
}

/// What [`ensure`] does, given the `user.reg` on disk and whether the launch
/// boots a fresh session.
fn plan<'a>(user_reg: Option<&str>, fresh: bool, wanted: &'a [Setting]) -> Plan<'a> {
    let missing: Vec<&Setting> = wanted
        .iter()
        .filter(|setting| !user_reg.is_some_and(|text| has_value(text, setting)))
        .collect();
    if missing.is_empty() {
        return Plan::Nothing;
    }
    match user_reg {
        Some(text) if fresh => Plan::WriteFile {
            text: missing
                .iter()
                .fold(text.to_owned(), |text, setting| set_value(&text, setting)),
            missing,
        },
        _ => Plan::RegAdd(missing),
    }
}

/// Put the values in place for the session the launch goes into. Consumes
/// the session so a held lock is released here, before the game's wine runs.
pub fn ensure(
    prefix: &Path,
    wine: &Path,
    wineserver: &Path,
    game_dir: &Path,
    retina: bool,
    dpi: u32,
    session: Session,
) {
    let user_reg = prefix.join("user.reg");
    let contents = fs::read_to_string(&user_reg).ok();
    let wanted = settings(retina, dpi);
    let fresh = matches!(session, Session::Fresh(_));
    let missing = match plan(contents.as_deref(), fresh, &wanted) {
        Plan::Nothing => {
            info!("user.reg already holds the wanted values, skipping registry setup");
            return;
        }
        // Written while the lock is held, see the module comment.
        Plan::WriteFile { text, missing } => match write_atomic(&user_reg, &text) {
            Ok(()) => {
                for setting in &missing {
                    info!(
                        "wrote {}\\{}={} into user.reg (no wineserver running)",
                        setting.hkcu_key(),
                        setting.name,
                        setting.reg_text
                    );
                }
                return;
            }
            Err(e) => {
                warn!("{e}; going through reg add");
                missing
            }
        },
        Plan::RegAdd(missing) => missing,
    };
    // reg add boots a session when none is up, and it needs the lock free.
    drop(session);

    for setting in missing {
        set_registry_value(wine, game_dir, setting);
    }

    if fresh {
        // The session reg add booted started before the values were in the
        // registry, so its desktop runs without them. Stop it: the game's
        // session then boots from the user.reg the shutdown saves.
        info!("stopping the wine session reg add booted, so the game's boots with the values");
        if let Err(e) = session::stop_server(wineserver) {
            warn!("could not stop the wine session reg add booted: {e}");
        }
    }
}

fn set_registry_value(wine: &Path, game_dir: &Path, setting: &Setting) {
    let key = setting.hkcu_key();
    info!(
        "Setting Wine registry value {key}\\{}={}",
        setting.name, setting.value
    );
    let result = Command::new(wine)
        .args([
            "reg",
            "add",
            &key,
            "/v",
            setting.name,
            "/t",
            setting.kind,
            "/d",
            &setting.value,
            "/f",
        ])
        .current_dir(game_dir)
        .output();

    match result {
        Ok(out) if out.status.success() => {
            info!(
                "Wine registry value {key}\\{} set successfully",
                setting.name
            );
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            warn!(
                "reg add for {key}\\{} exited with status {}: {}{}",
                setting.name, out.status, stdout, stderr,
            );
        }
        Err(e) => warn!("failed to run reg add for {key}\\{}: {e}", setting.name),
    }
}

/// Whether `user.reg` already holds the value, in its key.
fn has_value(user_reg: &str, setting: &Setting) -> bool {
    section(user_reg, setting.key).is_some_and(|(start, end)| {
        user_reg[start..end]
            .lines()
            .any(|line| line.trim_end() == setting.line())
    })
}

/// `user.reg` with the value set: replaced in its key, added to the key, or
/// added with the key at the end. Everything else is left byte for byte.
fn set_value(user_reg: &str, setting: &Setting) -> String {
    let line = setting.line();
    let name_prefix = format!("\"{}\"=", setting.name);
    let Some((start, end)) = section(user_reg, setting.key) else {
        let now = session::now_secs();
        let before = user_reg.trim_end_matches('\n');
        let gap = if before.is_empty() { "" } else { "\n\n" };
        return format!("{before}{gap}[{}] {now}\n{line}\n", setting.key);
    };

    let body = &user_reg[start..end];
    let mut out = String::with_capacity(user_reg.len() + line.len() + 1);
    out.push_str(&user_reg[..start]);
    let mut replaced = false;
    let mut last_meta = 0;
    let mut lines: Vec<&str> = body.lines().collect();
    for (i, existing) in lines.iter_mut().enumerate() {
        if existing.starts_with(&name_prefix) {
            *existing = &line;
            replaced = true;
        } else if existing.starts_with('[') || existing.starts_with('#') {
            last_meta = i + 1;
        }
    }
    if !replaced {
        lines.insert(last_meta, &line);
    }
    for existing in lines {
        out.push_str(existing);
        out.push('\n');
    }
    out.push_str(&user_reg[end..]);
    out
}

/// Byte range of the key's section: its header line through the last line
/// before the next header, including the trailing newline of each line.
fn section(user_reg: &str, key: &str) -> Option<(usize, usize)> {
    let header = format!("[{key}]");
    let mut offset = 0;
    let mut start = None;
    for line in user_reg.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if let Some(s) = start {
            if trimmed.starts_with('[') {
                return Some((s, offset));
            }
        } else if trimmed == header || trimmed.starts_with(&format!("{header} ")) {
            start = Some(offset);
        }
        offset += line.len();
    }
    start.map(|s| (s, user_reg.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "WINE REGISTRY Version 2\n;; All keys relative to REGISTRY\\\\User\\\\S-1-5-21-0-0-0-1000\n\n#arch=win64\n\n[Software\\\\Wine\\\\Mac Driver] 1787431559\n#time=1dd32773d2e3cd4\n\"AllowSetGamma\"=dword:00000000\n\"RetinaMode\"=\"N\"\n\n[Software\\\\Wine\\\\WineDbg] 1787411048\n#time=1dd32477b50b3e6\n\"ShowCrashDialog\"=dword:00000000\n\n[Software\\\\Wine\\\\X11 Driver] 1787936195\n#time=1dd370e2f8fea46\n\"EmulateModeset\"=\"Y\"\n\n[Volatile Environment] 1787428748\n\"TEMP\"=\"C:\\\\users\\\\player\\\\Temp\"\n";

    #[test]
    fn finds_values_in_their_own_key_only() {
        let [retina_off, _, crash_off, emulate] = settings(false, 96);
        assert!(has_value(SAMPLE, &retina_off));
        assert!(has_value(SAMPLE, &crash_off));
        assert!(has_value(SAMPLE, &emulate));
        let [retina_on, _, _, _] = settings(true, 96);
        assert!(!has_value(SAMPLE, &retina_on));
        // The same line under another key does not count.
        let elsewhere = "[Other] 1\n\"RetinaMode\"=\"Y\"\n";
        assert!(!has_value(elsewhere, &retina_on));
        let without_emulate = SAMPLE.replace("\"EmulateModeset\"=\"Y\"\n", "");
        assert!(!has_value(&without_emulate, &emulate));
    }

    #[test]
    fn replaces_a_value_in_place_and_leaves_the_rest() {
        let [retina_on, _, _, _] = settings(true, 96);
        let updated = set_value(SAMPLE, &retina_on);
        assert_eq!(
            updated,
            SAMPLE.replace("\"RetinaMode\"=\"N\"", "\"RetinaMode\"=\"Y\"")
        );
        assert!(has_value(&updated, &retina_on));
    }

    #[test]
    fn adds_a_value_after_the_keys_metadata() {
        let without = SAMPLE.replace("\"RetinaMode\"=\"N\"\n", "");
        let [retina_on, _, _, _] = settings(true, 96);
        let updated = set_value(&without, &retina_on);
        assert!(updated.contains(
            "[Software\\\\Wine\\\\Mac Driver] 1787431559\n#time=1dd32773d2e3cd4\n\"RetinaMode\"=\"Y\"\n\"AllowSetGamma\"=dword:00000000\n"
        ));
        assert!(has_value(&updated, &retina_on));
    }

    #[test]
    fn adds_a_missing_key_at_the_end() {
        let without = "WINE REGISTRY Version 2\n\n[Other] 1\n\"A\"=\"b\"\n";
        let [_, _, crash_off, _] = settings(false, 96);
        let updated = set_value(without, &crash_off);
        assert!(updated.starts_with(without));
        assert!(updated.contains("\n[Software\\\\Wine\\\\WineDbg] "));
        assert!(updated.ends_with("\"ShowCrashDialog\"=dword:00000000\n"));
        assert!(has_value(&updated, &crash_off));
        // Only one blank line separates it from the key before.
        assert!(updated.contains("\"A\"=\"b\"\n\n[Software"));
    }

    #[test]
    fn the_dpi_goes_in_the_desktop_key_and_not_wines_font_cache() {
        let [_, dpi, _, _] = settings(true, 192);
        assert_eq!(dpi.line(), "\"LogPixels\"=dword:000000c0");
        // A real prefix also holds the value under Wine's own Fonts key, where
        // the font code records the DPI its cache was built for. That copy is
        // not the setting.
        let cached = "[Software\\\\Wine\\\\Fonts] 1\n\"LogPixels\"=dword:000000c0\n\n\
                      [Control Panel\\\\Desktop] 1\n#time=1\n\"ActiveWndTrackTimeout\"=dword:00000000\n";
        assert!(!has_value(cached, &dpi));
        let updated = set_value(cached, &dpi);
        assert!(updated.contains(
            "[Control Panel\\\\Desktop] 1\n#time=1\n\"LogPixels\"=dword:000000c0\n\"ActiveWndTrackTimeout\""
        ));
        assert!(has_value(&updated, &dpi));
        let [_, unscaled, _, _] = settings(true, 96);
        assert!(!has_value(&updated, &unscaled));
    }

    #[test]
    fn plans_nothing_a_file_write_or_reg_add() {
        let complete = set_value(SAMPLE, &Setting::log_pixels(96));
        let unchanged = settings(false, 96);
        assert_eq!(plan(Some(&complete), true, &unchanged), Plan::Nothing);
        assert_eq!(plan(Some(&complete), false, &unchanged), Plan::Nothing);

        // Only what is missing is written or added.
        let retina_on = settings(true, 96);
        assert_eq!(
            plan(Some(&complete), true, &retina_on),
            Plan::WriteFile {
                text: complete.replace("\"RetinaMode\"=\"N\"", "\"RetinaMode\"=\"Y\""),
                missing: vec![&retina_on[0]],
            }
        );
        // A running session has loaded user.reg already.
        assert_eq!(
            plan(Some(&complete), false, &retina_on),
            Plan::RegAdd(vec![&retina_on[0]])
        );
        // A prefix wine has not created yet has nothing to write into.
        let wanted = settings(false, 96);
        assert_eq!(
            plan(None, true, &wanted),
            Plan::RegAdd(wanted.iter().collect())
        );
    }

    #[test]
    fn the_last_key_runs_to_the_end_of_the_file() {
        let text = "[Software\\\\Wine\\\\WineDbg] 1\n\"X\"=\"y\"";
        let (start, end) = section(text, r"Software\\Wine\\WineDbg").unwrap();
        assert_eq!(&text[start..end], text);
        assert!(section(text, r"Software\\Wine\\Mac Driver").is_none());
    }

    #[test]
    fn reg_add_key_uses_single_backslashes() {
        let [retina, dpi, crash, emulate] = settings(true, 192);
        assert_eq!(retina.hkcu_key(), r"HKCU\Software\Wine\Mac Driver");
        assert_eq!(dpi.hkcu_key(), r"HKCU\Control Panel\Desktop");
        assert_eq!(dpi.value, "192");
        assert_eq!(dpi.kind, "REG_DWORD");
        assert_eq!(crash.hkcu_key(), r"HKCU\Software\Wine\WineDbg");
        assert_eq!(emulate.hkcu_key(), r"HKCU\Software\Wine\X11 Driver");
        assert_eq!(retina.value, "Y");
        assert_eq!(crash.value, "0");
        assert_eq!(emulate.value, "Y");
        assert_eq!(emulate.kind, "REG_SZ");
    }
}
