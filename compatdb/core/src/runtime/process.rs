//! The process side: its arch, and the UTF-16 image path, command line and
//! environment block as wine hands them over.

use crate::{EnvEntry, part::steam_app_id};

/// The variable of the environment block that holds a process's Steam app id,
/// which [`steam_app`] reads.
pub const STEAM_APP_VAR: &str = "SteamAppId";

/// The architecture of the program a process runs, which is that of its main
/// executable.
///
/// Wine itself, the unix side with `ntdll.so` and this library, is built for
/// arm64 and runs natively; x86 code is emulated inside the process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    /// A 64-bit x86 program. It runs in an ARM64EC process with its x86 code
    /// emulated.
    X86_64,
    /// A 32-bit x86 program, run through wow64 with its x86 code emulated.
    I386,
    /// A native ARM64 program.
    Aarch64,
}

impl Arch {
    /// The `<arch>-windows` builtin directory name. The builtins of an
    /// `x86_64` process load from `aarch64-windows`, as upstream Wine does on
    /// an arm64 host: they are ARM64X modules that serve the process's
    /// ARM64EC side.
    #[must_use]
    pub const fn pe_dir(self) -> &'static str {
        match self {
            Self::I386 => "i386-windows",
            Self::X86_64 | Self::Aarch64 => "aarch64-windows",
        }
    }

    /// The arch of the current process. `wow64` says whether it is a wow64
    /// (32-bit) process; `machine` is the `Machine` field of its main image's
    /// COFF header, when it could be read.
    ///
    /// The bundle only runs x86 programs under wow64, so a wow64 process is
    /// i386. A 64-bit process is always native from Wine's side (ARM64EC for
    /// an `x86_64` program), so the main image decides:
    /// `IMAGE_FILE_MACHINE_ARM64` is a native ARM64 program, and anything
    /// else, an unreadable header included, counts as `x86_64`. An ARM64EC
    /// executable carries the AMD64 machine in its header and so also counts
    /// as `x86_64`, which is what it presents itself as.
    #[must_use]
    pub const fn of_process(wow64: bool, machine: Option<u16>) -> Self {
        match (wow64, machine) {
            (true, _) => Self::I386,
            (false, Some(IMAGE_FILE_MACHINE_ARM64)) => Self::Aarch64,
            _ => Self::X86_64,
        }
    }
}

/// The COFF `Machine` value of an ARM64 image.
pub const IMAGE_FILE_MACHINE_ARM64: u16 = 0xaa64;

/// The basename of a DOS image path (`...\Foo\Bar.exe` -> `Bar.exe`), decoded
/// lossily from UTF-16.
#[must_use]
pub fn basename_of(image_path: &[u16]) -> String {
    let start = image_path
        .iter()
        .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
        .map_or(0, |i| i.saturating_add(1));
    String::from_utf16_lossy(image_path.get(start..).unwrap_or(&[]))
}

/// Append each text to the command line unless it is already present.
///
/// The substring test is case-sensitive, the rule that leaves Chromium
/// `--type=` children, which inherit their parent's switches, untouched.
/// Returns `None` when nothing changed.
#[must_use]
pub fn append_cmdline(cmdline: &[u16], appends: &[impl AsRef<str>]) -> Option<Vec<u16>> {
    append_cmdline_each(cmdline, appends).0
}

/// [`append_cmdline`], also saying for each text whether it was appended:
/// false for one that was already present, or empty.
#[must_use]
pub fn append_cmdline_each(
    cmdline: &[u16],
    appends: &[impl AsRef<str>],
) -> (Option<Vec<u16>>, Vec<bool>) {
    let mut cur = cmdline.to_vec();
    let mut appended = Vec::with_capacity(appends.len());
    for text in appends {
        let text = text.as_ref();
        if text.is_empty() || contains_cs(&cur, text) {
            appended.push(false);
            continue;
        }
        cur.push(u16::from(b' '));
        cur.extend(text.encode_utf16());
        appended.push(true);
    }
    (appended.contains(&true).then_some(cur), appended)
}

/// The value of the variable `name` in a `NAME=value\0...\0\0` block, the
/// first entry for it, matched case-insensitively and decoded lossily; `None`
/// when the block has none.
#[must_use]
pub fn env_value(block: &[u16], name: &str) -> Option<String> {
    let seg = split_env(block)
        .into_iter()
        .find(|seg| env_name_matches(seg, name))?;
    let value = seg
        .iter()
        .position(|&u| u == u16::from(b'='))
        .and_then(|i| seg.get(i.saturating_add(1)..))
        .unwrap_or(&[]);
    Some(String::from_utf16_lossy(value))
}

/// The Steam app id in a `NAME=value\0...\0\0` block, the value of
/// [`STEAM_APP_VAR`]. A value that is empty, 0 or no number is no app id:
/// Steam's own processes have the variable empty.
///
/// It has to come from the Windows block: Wine does not copy that to the unix
/// environment, so `getenv` does not see what Steam sets for a game.
#[must_use]
pub fn steam_app(block: &[u16]) -> Option<u32> {
    steam_app_id(&env_value(block, STEAM_APP_VAR)?)
}

/// Merge environment entries into a `NAME=value\0...\0\0` block.
///
/// A name is matched case-insensitively; a value replaces in place or
/// appends, an entry without one removes. Drive entries (`=C:=...`) are left
/// alone. Returns `None` when nothing changed.
#[must_use]
pub fn merge_env(block: &[u16], entries: &[EnvEntry]) -> Option<Vec<u16>> {
    let mut segs = split_env(block);
    let mut changed = false;
    for entry in entries {
        let name = entry.name().as_str();
        let found = segs.iter().position(|seg| env_name_matches(seg, name));
        let Some(value) = entry.value() else {
            if let Some(i) = found {
                segs.remove(i);
                changed = true;
            }
            continue;
        };
        let mut new_seg: Vec<u16> = name.encode_utf16().collect();
        new_seg.push(u16::from(b'='));
        new_seg.extend(value.encode_utf16());
        match found {
            Some(i) if segs.get(i) == Some(&new_seg) => {}
            Some(i) => {
                if let Some(slot) = segs.get_mut(i) {
                    *slot = new_seg;
                    changed = true;
                }
            }
            None => {
                segs.push(new_seg);
                changed = true;
            }
        }
    }
    if !changed {
        return None;
    }
    let mut out = Vec::new();
    for seg in &segs {
        out.extend_from_slice(seg);
        out.push(0);
    }
    out.push(0);
    Some(out)
}

/// The UTF-16 unit `u` in lower case when it is an ASCII capital letter.
pub const fn to_lower_u16(u: u16) -> u16 {
    if u >= b'A' as u16 && u <= b'Z' as u16 {
        u | 0x20
    } else {
        u
    }
}

/// Case-sensitive substring test.
fn contains_cs(haystack: &[u16], needle: &str) -> bool {
    let n: Vec<u16> = needle.encode_utf16().collect();
    if n.is_empty() {
        return true;
    }
    if haystack.len() < n.len() {
        return false;
    }
    haystack.windows(n.len()).any(|w| w == n.as_slice())
}

/// Split an environment block into its `NAME=value` segments, dropping the
/// terminating NULs.
fn split_env(block: &[u16]) -> Vec<Vec<u16>> {
    let mut segs = Vec::new();
    let mut cur = Vec::new();
    for &u in block {
        if u == 0 {
            if cur.is_empty() {
                break;
            }
            segs.push(std::mem::take(&mut cur));
        } else {
            cur.push(u);
        }
    }
    segs
}

/// Whether an env segment's name equals `name` case-insensitively, skipping
/// the `=C:` drive entries.
fn env_name_matches(seg: &[u16], name: &str) -> bool {
    if seg.first() == Some(&u16::from(b'=')) {
        return false;
    }
    let seg_name = seg.split(|&u| u == u16::from(b'=')).next().unwrap_or(seg);
    let want: Vec<u16> = name.encode_utf16().map(to_lower_u16).collect();
    seg_name.len() == want.len()
        && seg_name
            .iter()
            .map(|&u| to_lower_u16(u))
            .eq(want.iter().copied())
}

#[cfg(test)]
mod tests;
