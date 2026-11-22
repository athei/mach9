//! Resolve a child's app name without applying any of its runtime settings.

use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path};

use compatdb_core::{
    Table, VersionInfo,
    runtime::{basename_of, steam_app},
};

use crate::{RULE_SOURCES, load_table, ntdll::Ntdll, version};

/// Look up a name during initial image selection or before forking a Windows
/// child. Use the target's environment: a game launched by Steam can have a
/// `SteamAppId` that the Steam client itself does not. This lookup must not
/// apply the target's settings to the process asking for its name.
///
/// Counts are UTF-16 code units, not bytes. The counted DOS image path
/// excludes NUL; the environment includes its final empty entry (an empty
/// environment may also be passed as null with length zero).
///
/// Return the number of UTF-8 bytes written, excluding the terminating NUL,
/// or zero without writing on no match, invalid input, or insufficient space.
/// The result is display text, not a path component.
///
/// # Safety
/// Nonempty input regions must be readable, initialized, aligned and valid for
/// their counts. The output must be writable for its count and not overlap
/// either input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compatdb_resolve_display_name(
    image: *const u16,
    image_len: usize,
    env: *const u16,
    env_len: usize,
    out: *mut libc::c_char,
    out_size: usize,
) -> usize {
    if image_len == 0
        || image.is_null()
        || !image.is_aligned()
        || image_len > isize::MAX as usize / size_of::<u16>()
        || env_len > isize::MAX as usize / size_of::<u16>()
        || (env_len != 0 && (env.is_null() || !env.is_aligned()))
        || out.is_null()
        || out_size == 0
        || out_size > isize::MAX as usize
    {
        return 0;
    }
    // SAFETY: the caller provides valid regions; checked the slice size,
    // nullness and alignment above. A zero-length env need not have a pointer.
    let (image, env) = unsafe {
        (
            std::slice::from_raw_parts(image, image_len),
            if env_len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(env, env_len)
            },
        )
    };
    if image.contains(&0) || !valid_env(env) {
        return 0;
    }
    let exe = basename_of(image);
    if exe.is_empty() {
        return 0;
    }

    let Some(sources) = RULE_SOURCES.get() else {
        return 0;
    };
    let (table, _) = load_table(sources);
    let Some(resolution) = resolve(&table, &exe, env, || image_version(image)) else {
        return 0;
    };
    let Some(name) = resolution
        .display_name()
        .filter(|name| name.len() < out_size)
    else {
        return 0;
    };
    // SAFETY: the caller supplies non-overlapping writable storage. The title
    // (which cannot contain NUL) plus its terminator fits, as checked above.
    unsafe {
        std::ptr::copy_nonoverlapping(name.as_ptr(), out.cast(), name.len());
        out.add(name.len()).write(0);
    }
    name.len()
}

fn valid_env(env: &[u16]) -> bool {
    match env {
        [] | [0] | [0, 0] => true,
        [entries @ .., 0, 0] => {
            entries.first() != Some(&0)
                && entries.last() != Some(&0)
                && !entries.windows(2).any(|pair| pair == [0, 0])
        }
        _ => false,
    }
}

fn resolve(
    table: &Table,
    exe: &str,
    env: &[u16],
    image_version: impl FnOnce() -> Option<VersionInfo>,
) -> Option<compatdb_core::Resolution> {
    let steam = steam_app(env);
    let mut naming_rules = table
        .rules()
        .iter()
        .filter(|rule| !rule.display_name().is_unset() && rule.exe().matches(exe, steam));
    let first = naming_rules.next()?;
    let version =
        if first.has_fingerprint() || naming_rules.any(compatdb_core::Rule::has_fingerprint) {
            image_version()
        } else {
            None
        };
    Some(table.resolve_with_optional_version(exe, version.as_ref(), steam))
}

fn image_version(image: &[u16]) -> Option<VersionInfo> {
    let path = Ntdll::resolve()?.unix_file_name(image).ok()?;
    version::read_file(Path::new(OsStr::from_bytes(&path)))
}

#[cfg(test)]
mod tests;
