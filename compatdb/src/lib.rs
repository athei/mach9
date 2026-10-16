//! `compatdb.so`: the per-process settings library wine's ntdll dlopens from
//! `<ntdll_dir>/compatdb.so` in every process (the slot `CrossOver` hack 24067
//! provides), before any PE code runs. Its main entry point is a Mach-O
//! initializer; ntdll also calls `compatdb_query_x87` to decide whether a
//! process starts under the x87sidecar (only in the `x86_64` library, since
//! the sidecar is for Rosetta), and [`compatdb_query_env`] for the rule
//! environment to set before it execs a process.
//!
//! The same source builds the library for the `x86_64` Wine and the arm64
//! one. What differs between them is data chosen by [`Host`]: the built-in
//! rules, the Direct3D defaults and the builtin directory of a 64-bit
//! process.
//!
//! It holds the built-in rules, overlays the ones passed in
//! [`ENV_VAR`](compatdb_table::ENV_VAR), resolves the rules that match this
//! process and applies them: the D3D9 and DXGI trees, DLL load-order
//! overrides, the DPI awareness, large address awareness, and
//! command-line/environment rewrites. All the decision logic lives in
//! [`compatdb_table`]; this crate only moves bytes in and out of the running
//! process, so its `unsafe` is confined to a thin ntdll shell.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![forbid(clippy::undocumented_unsafe_blocks)]

mod apply;
mod builtin;
mod log;
mod macos;
mod ntdll;
mod version;

use std::{
    ffi::{CStr, CString, OsStr, c_char, c_int},
    fmt::Write as _,
    os::unix::ffi::OsStrExt as _,
    path::Path,
};

use compatdb_table::{
    Arch, D3d9, Dxgi, ENV_VAR, Host, Resolution, Table, VersionInfo, basename_of,
};

use crate::ntdll::{Ntdll, Peb, Peb32, ProcessParameters, ProcessParameters32, TEB_PEB_OFFSET};

/// The Mach-O initializer, run from ntdll's `dlopen` in every wine process.
#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static INIT: extern "C" fn() = init;

extern "C" fn init() {
    // ntdll can load the library before it is initialized itself, only to
    // call compatdb_query_x87 and compatdb_query_env, and says so through
    // compatdb_preinit_query.
    // Nothing here would work then, so do nothing. dyld does not unload this
    // library on dlclose, and a later dlopen of it does not run INIT again,
    // so skipping here would also skip the real run in this process. That is
    // only correct because ntdll's pre-init path always ends in execv or
    // fatal_error: the process that runs the program is a fresh image that
    // loads the library again and runs INIT normally.
    if ntdll::is_preinit_load() {
        return;
    }
    run();
}

/// Whether the executable at `unix_path` should start under the x87sidecar:
/// 1 to attach it, 0 not to, -1 for no opinion (a null or empty path, or no
/// rule that sets `x87_sidecar`).
///
/// ntdll calls this before it starts an i386 process, and possibly before
/// ntdll itself is initialized, so it reads only the rule table and the file:
/// no PEB, no TEB and no ntdll symbol. It keeps no state between calls. The
/// path is a unix path when ntdll could resolve one; otherwise it may be a DOS
/// path, in which case only the basename is known and the version fingerprint
/// is empty.
///
/// Only the `x86_64` library exports it: the sidecar exists for Rosetta,
/// which the arm64 Wine does not run under.
///
/// # Safety
/// `unix_path` must be null or point at a NUL-terminated string that stays
/// valid for the duration of the call.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compatdb_query_x87(unix_path: *const c_char) -> c_int {
    if unix_path.is_null() {
        log::line("x87 query without a path, no opinion");
        return -1;
    }
    // SAFETY: the caller passes a NUL-terminated string that stays valid for
    // the call; it is copied out before anything else happens.
    let path = unsafe { CStr::from_ptr(unix_path) }.to_bytes().to_vec();
    let (table, _) = load_table();
    let Some((exe, version, resolution)) = query(&table, &path) else {
        log::line("x87 query without a file name, no opinion");
        return -1;
    };
    let (answer, ret) = match resolution.x87_sidecar {
        Some(true) => ("x87_sidecar (rule) = true", 1),
        Some(false) => ("x87_sidecar (rule) = false", 0),
        None => ("x87_sidecar (rule) unset, no opinion", -1),
    };
    log::line(&format!(
        "x87 query {exe}{}: {answer}",
        fingerprint(&version)
    ));
    ret
}

/// The rule environment of the executable at `path`, for ntdll to set in the
/// unix environment right before it execs the process.
///
/// The process gets its environment block and unix environment from compatdb
/// only once it runs, which is too late for what starts ahead of it: the
/// x87sidecar reads its `X87_*` variables before it execs Wine, and Rosetta
/// reads its own at exec.
///
/// Writes the entries into `buf` as `NAME=value\0` strings in resolution
/// order, so a later entry for the same name wins, followed by one more `\0`.
/// An empty value means the variable is to be removed. Entries whose name is
/// empty or holds `=` or NUL, or whose value holds NUL, are left out.
///
/// Returns the number of bytes the entries need, including the final `\0`.
/// When that is more than `size`, or `buf` is null, nothing is written and the
/// caller retries with a buffer that large. Returns 0 when no entry applies
/// (no rule sets one, or the path names no file) and -1 for a null path or
/// entries too large for an `int`.
///
/// The path is a unix path or a DOS one, as for `compatdb_query_x87`, under
/// the same constraints: this can run before ntdll is initialized and keeps
/// no state.
/// ntdll may `dlclose` the library right after a pre-init query, which is why
/// the answer goes into the caller's buffer and nothing points into this
/// library.
///
/// # Safety
/// `path` must be null or point at a NUL-terminated string that stays valid
/// for the duration of the call. `buf` must be null or point at `size`
/// writable bytes that do not overlap `path`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compatdb_query_env(
    path: *const c_char,
    buf: *mut c_char,
    size: usize,
) -> c_int {
    if path.is_null() {
        log::line("env query without a path");
        return -1;
    }
    // SAFETY: the caller passes a NUL-terminated string that stays valid for
    // the call; it is copied out before anything else happens.
    let path = unsafe { CStr::from_ptr(path) }.to_bytes().to_vec();
    let (table, _) = load_table();
    // SAFETY: the caller guarantees `buf` is null or covers `size` writable
    // bytes.
    unsafe { answer_env(&table, &path, buf, size) }
}

/// [`compatdb_query_env`] against `table`, once the path is copied out.
///
/// # Safety
/// `buf` must be null or point at `size` writable bytes.
unsafe fn answer_env(table: &Table, path: &[u8], buf: *mut c_char, size: usize) -> c_int {
    let Some((exe, version, env)) = query_env(table, path) else {
        return 0;
    };
    if env.is_empty() {
        return 0;
    }
    let block = encode_env(&env);
    let Ok(needed) = c_int::try_from(block.len()) else {
        log::line(&format!(
            "env query {exe}{}: entries too large to return",
            fingerprint(&version)
        ));
        return -1;
    };
    if buf.is_null() || block.len() > size {
        return needed;
    }
    // SAFETY: `buf` is non-null and covers `size` writable bytes, at least
    // `block.len()` of them; `block` is this function's own allocation, so
    // the two do not overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(block.as_ptr(), buf.cast::<u8>(), block.len());
    }
    let list: Vec<String> = env
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    log::line(&format!(
        "env query {exe}{}: {}",
        fingerprint(&version),
        list.join(", ")
    ));
    needed
}

/// `NAME`/value pairs as [`Resolution::env`] holds them.
type EnvEntries = Vec<(String, String)>;

/// Resolve the `env` entries for the executable at `path`, a unix path or a
/// DOS one, leaving out the ones that cannot go into a unix environment.
/// Returns the basename and version fingerprint it matched on along with the
/// entries in resolution order, or `None` when the path names no file.
fn query_env(table: &Table, path: &[u8]) -> Option<(String, VersionInfo, EnvEntries)> {
    let (exe, version, resolution) = query(table, path)?;
    let env = resolution
        .env
        .into_iter()
        .filter(|(name, value)| {
            !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0')
        })
        .collect();
    Some((exe, version, env))
}

/// `NAME=value\0` for each entry, then one more `\0`; empty for no entries.
/// The entries must already be free of NUL, and the names of `=`.
fn encode_env(entries: &[(String, String)]) -> Vec<u8> {
    if entries.is_empty() {
        return Vec::new();
    }
    let mut block = Vec::new();
    for (name, value) in entries {
        block.extend_from_slice(name.as_bytes());
        block.push(b'=');
        block.extend_from_slice(value.as_bytes());
        block.push(0);
    }
    block.push(0);
    block
}

/// Resolve the rules for the executable at `path`, a unix path or a DOS one.
/// Returns the basename and version fingerprint it matched on along with the
/// resolution, or `None` when the path names no file.
fn query(table: &Table, path: &[u8]) -> Option<(String, VersionInfo, Resolution)> {
    let path_utf16: Vec<u16> = String::from_utf8_lossy(path).encode_utf16().collect();
    let exe = basename_of(&path_utf16);
    if exe.is_empty() {
        return None;
    }
    // ntdll passes an absolute unix path for a file it found. Anything else
    // is a DOS path or a bare name, which must not be opened relative to the
    // current directory, so it reads as no version resource, as does a file
    // that does not exist.
    let version = if path.starts_with(b"/") {
        version::read_file(Path::new(OsStr::from_bytes(path)))
    } else {
        VersionInfo::default()
    };
    let resolution = table.resolve(&exe, &version);
    Some((exe, version, resolution))
}

/// The built-in rules with the overrides from [`ENV_VAR`] laid on top, plus
/// the lines worth logging about the overrides: parse diagnostics and
/// overrides that name no rule.
fn load_table() -> (Table, Vec<String>) {
    // The built-in rules are compiled in; only overrides travel in the
    // environment (absent unless something set it). Overlay them.
    let mut table = builtin::table();
    let mut messages = Vec::new();
    if let Some(text) = read_env() {
        let (overrides, diagnostics) = Table::parse(&text);
        messages.extend(diagnostics);
        for name in table.overlay(overrides) {
            messages.push(format!("override names no rule: {name}"));
        }
    }
    (table, messages)
}

fn run() {
    // The library always loads, in every wine process, whether or not the env
    // var is set: the default dirs ship no real Direct3D at all, so both
    // implementations are chosen here (per arch, or per the database) and
    // their trees prepended. It always logs one line per process so a miss is
    // as visible as a hit.
    let Some(nt) = Ntdll::resolve() else {
        log::line("required ntdll symbols missing, doing nothing");
        return;
    };
    let (table, messages) = load_table();
    for m in &messages {
        log::line(m);
    }
    let host = Host::current();
    let Some(process) = Process::current(&nt, host) else {
        return;
    };
    let resolution = table.resolve(&process.exe, &process.version);
    let arch = arch_name(process.arch, host);
    log::line(&format!(
        "{} [{arch}]{}",
        process.exe,
        fingerprint(&process.version)
    ));
    if resolution.matched.is_empty() {
        // Diagnose a miss by the fingerprint on the line above: an empty one
        // means the image carries no version resource for a rule to pin.
        log::line("  no rule matched");
    } else {
        log::line(&format!("  rules = {}", resolution.matched.join(", ")));
    }

    // Every process gets one tree of each family prepended: a rule's choice,
    // or the arch default, with dxmt in place of gptk where gptk cannot load,
    // and wined3d where neither dxmt nor mtld3d can (a native ARM64 program
    // in the arm64 Wine). Purely additive: the default dirs hold nothing that
    // could conflict, so nothing is ever disabled.
    // The macOS release matters only where some process could load gptk, so
    // a Wine without it neither reads nor logs it.
    let mac = Dxgi::Gptk
        .available_for(Arch::X86_64, host, true)
        .then(macos::host);
    let gptk_loads = mac.is_some_and(|m| m.gptk_loads);
    let release = mac.map_or("", |m| m.name.as_str());
    let dxgi = Dxgi::effective(resolution.dxgi, process.arch, host, gptk_loads);
    if let Some(note) = dxgi_fallback(resolution.dxgi, process.arch, host, release, dxgi) {
        log::line(&note);
    }
    apply::tree(&nt, process.arch.pe_dir(host), "dxgi", dxgi.as_str());
    let d3d9 = D3d9::effective(resolution.d3d9, process.arch, host);
    if let Some(note) = d3d9_fallback(resolution.d3d9, process.arch, host, d3d9) {
        log::line(&note);
    }
    apply::tree(&nt, process.arch.pe_dir(host), "d3d9", d3d9.as_str());

    // The remaining knobs apply only when a rule matched this process.
    if !resolution.dll_overrides.is_empty() {
        log::line(&format!(
            "  dll_overrides = {}",
            resolution.dll_overrides.join(";")
        ));
        apply::dll_overrides(&nt, &resolution.dll_overrides);
    }
    if let Some(aware) = resolution.dpi_aware {
        apply::dpi_awareness(&nt, aware);
    }
    if let Some(laa) = resolution.large_address_aware {
        apply::large_address_aware(&nt, laa);
    }
    // Only for the log, and only the rule's value: ntdll asked
    // compatdb_query_x87 before it started the process and decided then, and
    // it may have skipped the sidecar regardless (an empty ROSETTA_X87_PATH,
    // no sidecar installed, an ntdll without the query). For a first launch
    // by DOS path, ntdll resolves the path through the prefix's dosdevices;
    // when that fails the query saw only the basename. It means
    // nothing to a 64-bit process, so only an i386 one shows it, and nothing
    // in the arm64 Wine, which has no such query.
    #[cfg(target_arch = "x86_64")]
    if let (Some(x87), Arch::I386) = (resolution.x87_sidecar, process.arch) {
        log::line(&format!("  x87_sidecar (rule) = {x87}"));
    }
    if !resolution.arguments.is_empty() {
        log::line(&format!("  arguments = {}", resolution.arguments.join(" ")));
    }
    if !resolution.env.is_empty() {
        let env: Vec<String> = resolution
            .env
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        log::line(&format!("  env = {}", env.join(", ")));
    }
    // ntdll may already have set the same entries in the unix environment
    // before it exec'd the process (compatdb_query_env). Setting them again
    // is harmless and covers an ntdll without that query. The environment
    // block needs them either way.
    apply::rewrite_params(&nt, &process, &resolution);
    apply::unix_env(&resolution.env);
}

/// The arch of a process as the log prints it in the Wine built for `host`.
/// In the arm64 Wine an `x86_64` program runs in an ARM64EC process, which the
/// label says.
const fn arch_name(arch: Arch, host: Host) -> &'static str {
    match (arch, host) {
        (Arch::X86_64, Host::X86_64) => "x86_64",
        (Arch::X86_64, Host::Aarch64) => "x86_64 (arm64ec)",
        (Arch::I386, _) => "i386",
        (Arch::Aarch64, _) => "aarch64",
    }
}

/// The host as the log names it.
const fn host_name(host: Host) -> &'static str {
    match host {
        Host::X86_64 => "x86_64",
        Host::Aarch64 => "arm64",
    }
}

/// The log line for a process that gets another DXGI implementation than the
/// one it asked for (`requested`) or, without a rule, than the default for
/// its arch and host. `dxgi` is what [`Dxgi::effective`] chose and `release`
/// the macOS release it was decided on ([`macos::Host::name`]). `None` when
/// the process gets what it asked for.
fn dxgi_fallback(
    requested: Option<Dxgi>,
    arch: Arch,
    host: Host,
    release: &str,
    dxgi: Dxgi,
) -> Option<String> {
    let want = requested.unwrap_or_else(|| Dxgi::default_for(arch, host));
    if want == dxgi {
        return None;
    }
    // When the implementation exists for the arch (only gptk), the macOS
    // release is what keeps it from loading. When it exists for no arch of
    // this Wine (gptk in the arm64 one), the Wine is. Otherwise the arch is
    // (gptk for i386, dxmt for a native ARM64 program).
    let place = if want.available_for(arch, host, true) {
        format!("{release} (needs macOS {})", macos::GPTK_MIN_TEXT)
    } else if !want.available_for(Arch::X86_64, host, true) {
        host_name(host).to_owned()
    } else {
        arch_name(arch, host).to_owned()
    };
    Some(format!(
        "  dxgi = {} not available on {place}, using {}",
        want.as_str(),
        dxgi.as_str()
    ))
}

/// The log line for a process whose rule asks for a D3D9 implementation it
/// cannot load (mtld3d for a native ARM64 program in the arm64 Wine). `d3d9`
/// is what [`D3d9::effective`] chose. `None` when the process gets what it
/// asked for, or asked for nothing: the default always loads.
fn d3d9_fallback(requested: Option<D3d9>, arch: Arch, host: Host, d3d9: D3d9) -> Option<String> {
    let want = requested?;
    (want != d3d9).then(|| {
        format!(
            "  d3d9 = {} not available on {}, using {}",
            want.as_str(),
            arch_name(arch, host),
            d3d9.as_str()
        )
    })
}

/// A compact `company="..." product="..." original_filename="..."` suffix,
/// listing only the version fields that are present. Empty when the image
/// carries no version resource.
fn fingerprint(v: &VersionInfo) -> String {
    let mut parts = String::new();
    for (label, value) in [
        ("company", &v.company),
        ("product", &v.product),
        ("original_filename", &v.original_filename),
    ] {
        if !value.is_empty() {
            let _ = write!(parts, " {label}={value:?}");
        }
    }
    parts
}

/// The value of [`ENV_VAR`], read straight from the C environment so the
/// no-op path allocates nothing beyond the small variable name.
fn read_env() -> Option<String> {
    let name = CString::new(ENV_VAR).ok()?;
    // SAFETY: `name` is a valid NUL-terminated string; getenv returns a pointer
    // into the environment or null, and we copy it out immediately.
    let ptr = unsafe { libc::getenv(name.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: getenv returned a non-null pointer to a NUL-terminated string.
    let value = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    (!value.is_empty()).then_some(value)
}

/// An owned snapshot of the process identity and the two strings the library
/// may rewrite, taken once so all later work is on owned data.
pub(crate) struct Process {
    pub params: *mut ProcessParameters,
    pub params32: Option<*mut ProcessParameters32>,
    pub arch: Arch,
    pub exe: String,
    pub version: VersionInfo,
    pub cmdline: Vec<u16>,
    pub env: Vec<u16>,
}

impl Process {
    /// The current process, in the Wine built for `host`.
    fn current(nt: &Ntdll, host: Host) -> Option<Self> {
        let teb = nt.current_teb();
        if teb.is_null() {
            log::line("NtCurrentTeb returned null");
            return None;
        }
        // SAFETY: the TEB is page-aligned, and TEB.Peb sits at
        // TEB_PEB_OFFSET, a whole number of pointer-sized slots in (checked
        // in ntdll.rs), so the read is aligned. It holds the PEB pointer,
        // which ntdll populated before this library was loaded.
        let peb = unsafe {
            teb.cast::<*mut Peb>()
                .add(TEB_PEB_OFFSET / size_of::<*mut Peb>())
                .read()
        };
        if peb.is_null() {
            log::line("PEB pointer is null");
            return None;
        }
        // SAFETY: `peb` is the live PEB; ProcessParameters and the image base
        // are already set.
        let (params, image_base) = unsafe { ((*peb).process_parameters, (*peb).image_base) };
        if params.is_null() {
            log::line("ProcessParameters pointer is null");
            return None;
        }

        // ProcessWow64Information is the PEB32 address of a wow64 process and
        // 0 otherwise, on either host. Both Wines run only x86 programs under
        // wow64 (the arm64 one with an emulator), so such a process is i386.
        // A 64-bit process is x86_64 in the x86_64 Wine. In the arm64 Wine it
        // is native from Wine's side either way, ARM64EC for an x86_64
        // program, so the main image's machine tells an x86_64 program from a
        // native ARM64 one; see Arch::of_process. An x86_64 program stays
        // Arch::X86_64 there, for the log and the Direct3D defaults; the
        // rules themselves match on the image name and version alone.
        let params32 = match nt.wow64_information() {
            Some(w) if w != 0 => {
                let peb32 = w as *const Peb32;
                // SAFETY: a non-zero ProcessWow64Information value is the live
                // PEB32 address for a 32-bit process.
                let pp = unsafe { (*peb32).process_parameters };
                if pp == 0 {
                    log::line("wow64 ProcessParameters pointer is null");
                    return None;
                }
                Some(pp as usize as *mut ProcessParameters32)
            }
            _ => None,
        };
        let machine = version::machine(image_base.cast_const());
        let arch = Arch::of_process(host, params32.is_some(), machine);

        // SAFETY: `params` is the live 64-bit process parameters; each field
        // read below is a UNICODE_STRING or the environment pointer/size.
        let (image_path, cmdline, env) = unsafe {
            let p = &*params;
            (
                read_unicode(&p.image_path_name),
                read_unicode(&p.command_line),
                read_environment(p.environment, p.environment_size),
            )
        };
        let exe = basename_of(&image_path);
        let version = version::read(image_base.cast_const());

        Some(Self {
            params,
            params32,
            arch,
            exe,
            version,
            cmdline,
            env,
        })
    }
}

/// Copy a `UNICODE_STRING` into an owned `Vec<u16>`.
///
/// # Safety
/// `u.buffer` must be null or point at `u.length` bytes of UTF-16.
unsafe fn read_unicode(u: &ntdll::UnicodeString) -> Vec<u16> {
    if u.buffer.is_null() || u.length == 0 {
        return Vec::new();
    }
    let units = usize::from(u.length / 2);
    // SAFETY: the caller guarantees `buffer` covers `length` bytes.
    unsafe { std::slice::from_raw_parts(u.buffer, units) }.to_vec()
}

/// Copy an environment block into an owned `Vec<u16>`, keeping its terminators.
///
/// # Safety
/// `env` must be null or point at `size_bytes` bytes of UTF-16.
unsafe fn read_environment(env: *mut u16, size_bytes: usize) -> Vec<u16> {
    if env.is_null() || size_bytes < 2 {
        return vec![0, 0];
    }
    let units = size_bytes / 2;
    // SAFETY: the caller guarantees `env` covers `size_bytes` bytes.
    unsafe { std::slice::from_raw_parts(env, units) }.to_vec()
}

#[cfg(test)]
mod tests {
    use compatdb_table::{ANY_EXE, Rule};

    use super::*;

    fn table() -> Table {
        Table {
            rules: vec![
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    x87_sidecar: Some(true),
                    ..Rule::default()
                },
                Rule {
                    name: "no-x87".into(),
                    exe: "Old.exe".into(),
                    x87_sidecar: Some(false),
                    ..Rule::default()
                },
            ],
        }
    }

    /// What a test returns, so a missing value fails it through `?` rather
    /// than a panic in the test body.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// `x87_sidecar` for `path`, or an error when the path names no file.
    fn x87(table: &Table, path: &str) -> Result<Option<bool>, String> {
        query(table, path.as_bytes())
            .map(|(_, _, r)| r.x87_sidecar)
            .ok_or_else(|| format!("{path} names no file"))
    }

    #[test]
    fn the_query_matches_on_the_basename_of_a_unix_or_a_dos_path() -> TestResult {
        let table = table();
        assert_eq!(x87(&table, "/nonexistent/games/old.exe")?, Some(false));
        assert_eq!(x87(&table, "C:\\Games\\OLD.EXE")?, Some(false));
        assert_eq!(x87(&table, "old.exe")?, Some(false));
        assert_eq!(x87(&table, "/nonexistent/games/new.exe")?, Some(true));
        let (exe, version, _) = query(&table, b"C:\\Games\\Old.exe").ok_or("no file name")?;
        assert_eq!(exe, "Old.exe");
        assert_eq!(version, VersionInfo::default());
        Ok(())
    }

    #[test]
    fn the_query_has_no_opinion_without_a_file_name_or_a_rule() {
        let table = table();
        assert!(query(&table, b"").is_none());
        assert!(query(&table, b"/nonexistent/games/").is_none());
        assert!(query(&table, b"C:\\Games\\").is_none());
        assert_eq!(x87(&Table::default(), "/nonexistent/game.exe"), Ok(None));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_x87_export_has_no_opinion_without_a_file_name() {
        // SAFETY: a null path is part of the contract.
        assert_eq!(unsafe { compatdb_query_x87(std::ptr::null()) }, -1);
        let empty = c"";
        // SAFETY: a valid NUL-terminated string.
        assert_eq!(unsafe { compatdb_query_x87(empty.as_ptr()) }, -1);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_builtin_default_attaches_the_sidecar() {
        // The exported function reads WINE_COMPATDB, which a developer's shell
        // may set; the answer is only predictable without it.
        if std::env::var_os(ENV_VAR).is_some() {
            return;
        }
        let path = c"/nonexistent/games/game.exe";
        // SAFETY: a valid NUL-terminated string.
        assert_eq!(unsafe { compatdb_query_x87(path.as_ptr()) }, 1);
    }

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    fn env_table() -> Table {
        Table {
            rules: vec![
                Rule {
                    name: "game".into(),
                    exe: "Game.exe".into(),
                    env: vec![
                        pair("B", "2"),
                        pair("A", "3"),
                        pair("C", ""),
                        pair("", "empty-name"),
                        pair("D=E", "x"),
                        pair("F\0", "y"),
                        pair("G", "a\0b"),
                        pair("H", "x=y"),
                    ],
                    ..Rule::default()
                },
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    env: vec![pair("A", "1")],
                    ..Rule::default()
                },
            ],
        }
    }

    const GAME_BLOCK: &[u8] = b"A=1\0B=2\0A=3\0C=\0H=x=y\0\0";

    /// [`answer_env`] into a buffer of `size` bytes prefilled with 0xAA,
    /// returning the answer and the buffer.
    fn answer(table: &Table, path: &str, size: usize) -> (c_int, Vec<u8>) {
        let mut buf = vec![0xAA_u8; size];
        // SAFETY: `buf` covers `size` writable bytes.
        let ret = unsafe { answer_env(table, path.as_bytes(), buf.as_mut_ptr().cast(), size) };
        (ret, buf)
    }

    #[test]
    fn the_env_query_matches_on_the_basename_of_a_unix_or_a_dos_path() -> TestResult {
        let table = env_table();
        for path in [
            "/nonexistent/games/game.exe",
            "C:\\Games\\GAME.EXE",
            "Game.exe",
        ] {
            let (exe, version, env) = query_env(&table, path.as_bytes()).ok_or(path)?;
            assert_eq!(exe.to_ascii_lowercase(), "game.exe");
            assert_eq!(version, VersionInfo::default());
            assert_eq!(
                env,
                [
                    pair("A", "1"),
                    pair("B", "2"),
                    pair("A", "3"),
                    pair("C", ""),
                    pair("H", "x=y")
                ]
            );
        }
        let (_, _, env) = query_env(&table, b"/nonexistent/games/other.exe").ok_or("other")?;
        assert_eq!(env, [pair("A", "1")]);
        Ok(())
    }

    #[test]
    fn the_env_block_keeps_order_duplicates_and_empty_values() {
        let table = env_table();
        let (ret, buf) = answer(&table, "/nonexistent/games/game.exe", 64);
        assert_eq!(usize::try_from(ret).ok(), Some(GAME_BLOCK.len()));
        assert_eq!(buf.get(..GAME_BLOCK.len()), Some(GAME_BLOCK));
        assert!(
            buf.get(GAME_BLOCK.len()..)
                .is_some_and(|rest| rest.iter().all(|&b| b == 0xAA))
        );
        // Exactly the needed size is enough.
        let (ret, buf) = answer(&table, "C:\\Games\\Game.exe", GAME_BLOCK.len());
        assert_eq!(usize::try_from(ret).ok(), Some(GAME_BLOCK.len()));
        assert_eq!(buf, GAME_BLOCK);
        assert_eq!(encode_env(&[]), b"");
        assert_eq!(encode_env(&[pair("X", "")]), b"X=\0\0");
    }

    #[test]
    fn a_short_buffer_gets_nothing_and_learns_the_size() {
        let table = env_table();
        let (ret, buf) = answer(&table, "game.exe", GAME_BLOCK.len() - 1);
        assert_eq!(usize::try_from(ret).ok(), Some(GAME_BLOCK.len()));
        assert!(buf.iter().all(|&b| b == 0xAA));
        // SAFETY: a null buffer with size 0 is part of the contract.
        let ret = unsafe { answer_env(&table, b"game.exe", std::ptr::null_mut(), 0) };
        assert_eq!(usize::try_from(ret).ok(), Some(GAME_BLOCK.len()));
    }

    #[test]
    fn the_env_query_is_empty_without_a_match_a_file_name_or_a_path() {
        assert_eq!(answer(&env_table(), "", 64).0, 0);
        assert_eq!(answer(&env_table(), "C:\\Games\\", 64).0, 0);
        assert_eq!(answer(&table(), "/nonexistent/games/game.exe", 64).0, 0);
        assert_eq!(answer(&Table::default(), "game.exe", 64).0, 0);
        let invalid_only = Table {
            rules: vec![Rule {
                name: "invalid".into(),
                exe: ANY_EXE.into(),
                env: vec![pair("", "x"), pair("A=B", "x")],
                ..Rule::default()
            }],
        };
        let (ret, buf) = answer(&invalid_only, "game.exe", 64);
        assert_eq!(ret, 0);
        assert!(buf.iter().all(|&b| b == 0xAA));
        // SAFETY: a null path and a null buffer are part of the contract.
        let ret = unsafe { compatdb_query_env(std::ptr::null(), std::ptr::null_mut(), 0) };
        assert_eq!(ret, -1);
    }

    const INTEL: Host = Host::X86_64;
    const ARM: Host = Host::Aarch64;

    #[test]
    fn gptk_falls_back_to_dxmt_below_macos_26_4_and_says_why() {
        // The 64-bit default and an explicit gptk rule alike.
        for requested in [None, Some(Dxgi::Gptk)] {
            let old = macos::decide(Some("15.7.1"), false, Some("24.6.0"));
            let dxgi = Dxgi::effective(requested, Arch::X86_64, INTEL, old.gptk_loads);
            assert_eq!(dxgi, Dxgi::Dxmt);
            assert_eq!(
                dxgi_fallback(requested, Arch::X86_64, INTEL, &old.name, dxgi).as_deref(),
                Some("  dxgi = gptk not available on macOS 15.7.1 (needs macOS 26.4), using dxmt")
            );
            // Decided on the Darwin release when the product version is shimmed.
            let shimmed = macos::decide(Some("16.0"), true, Some("25.3.0"));
            let dxgi = Dxgi::effective(requested, Arch::X86_64, INTEL, shimmed.gptk_loads);
            assert_eq!(dxgi, Dxgi::Dxmt);
            assert_eq!(
                dxgi_fallback(requested, Arch::X86_64, INTEL, &shimmed.name, dxgi).as_deref(),
                Some("  dxgi = gptk not available on Darwin 25.3.0 (needs macOS 26.4), using dxmt")
            );
            for new in [
                macos::decide(Some("26.4"), false, Some("25.4.0")),
                macos::decide(Some("16.0"), true, Some("25.4.0")),
                // A version that cannot be read keeps gptk.
                macos::decide(None, false, None),
            ] {
                let dxgi = Dxgi::effective(requested, Arch::X86_64, INTEL, new.gptk_loads);
                assert_eq!(dxgi, Dxgi::Gptk);
                assert_eq!(
                    dxgi_fallback(requested, Arch::X86_64, INTEL, &new.name, dxgi),
                    None
                );
            }
        }
    }

    #[test]
    fn the_i386_fallback_line_names_the_arch_whatever_the_macos() {
        for gptk_loads in [true, false] {
            let dxgi = Dxgi::effective(Some(Dxgi::Gptk), Arch::I386, INTEL, gptk_loads);
            assert_eq!(dxgi, Dxgi::Dxmt);
            assert_eq!(
                dxgi_fallback(Some(Dxgi::Gptk), Arch::I386, INTEL, "macOS 15.7.1", dxgi).as_deref(),
                Some("  dxgi = gptk not available on i386, using dxmt")
            );
        }
        // The i386 default is dxmt already, and the other implementations
        // load everywhere, so neither logs a fallback.
        assert_eq!(
            dxgi_fallback(None, Arch::I386, INTEL, "macOS 15.7.1", Dxgi::Dxmt),
            None
        );
        for other in [Dxgi::Dxmt, Dxgi::Wined3d] {
            let dxgi = Dxgi::effective(Some(other), Arch::X86_64, INTEL, false);
            assert_eq!(dxgi, other);
            assert_eq!(
                dxgi_fallback(Some(other), Arch::X86_64, INTEL, "macOS 15.7.1", dxgi),
                None
            );
        }
    }

    #[test]
    fn the_arm64_wine_defaults_to_dxmt_quietly_and_names_itself_for_gptk() {
        // The macOS release is not read there, so it comes in empty. A native
        // ARM64 program is the exception, see the next test.
        for arch in [Arch::X86_64, Arch::I386] {
            let dxgi = Dxgi::effective(None, arch, ARM, false);
            assert_eq!(dxgi, Dxgi::Dxmt);
            assert_eq!(dxgi_fallback(None, arch, ARM, "", dxgi), None);
            // A gptk rule falls back, and the line blames the Wine, never the
            // macOS release.
            let dxgi = Dxgi::effective(Some(Dxgi::Gptk), arch, ARM, false);
            assert_eq!(dxgi, Dxgi::Dxmt);
            assert_eq!(
                dxgi_fallback(Some(Dxgi::Gptk), arch, ARM, "", dxgi).as_deref(),
                Some("  dxgi = gptk not available on arm64, using dxmt")
            );
            let dxgi = Dxgi::effective(Some(Dxgi::Wined3d), arch, ARM, false);
            assert_eq!(dxgi, Dxgi::Wined3d);
            assert_eq!(
                dxgi_fallback(Some(Dxgi::Wined3d), arch, ARM, "", dxgi),
                None
            );
        }
        // So only the x86_64 Wine reads the macOS release.
        assert!(Dxgi::Gptk.available_for(Arch::X86_64, INTEL, true));
        assert!(!Dxgi::Gptk.available_for(Arch::X86_64, ARM, true));
    }

    #[test]
    fn a_native_arm64_program_gets_wined3d_quietly_and_rules_say_why() {
        let native = Arch::Aarch64;
        // The default is wined3d for both families, with nothing to log.
        let dxgi = Dxgi::effective(None, native, ARM, false);
        assert_eq!(dxgi, Dxgi::Wined3d);
        assert_eq!(dxgi_fallback(None, native, ARM, "", dxgi), None);
        let d3d9 = D3d9::effective(None, native, ARM);
        assert_eq!(d3d9, D3d9::Wined3d);
        assert_eq!(d3d9_fallback(None, native, ARM, d3d9), None);
        // A rule for dxmt or mtld3d falls back, and the line names the arch;
        // one for gptk names the Wine, as for every arch there.
        let dxgi = Dxgi::effective(Some(Dxgi::Dxmt), native, ARM, false);
        assert_eq!(
            dxgi_fallback(Some(Dxgi::Dxmt), native, ARM, "", dxgi).as_deref(),
            Some("  dxgi = dxmt not available on aarch64, using wined3d")
        );
        let dxgi = Dxgi::effective(Some(Dxgi::Gptk), native, ARM, false);
        assert_eq!(
            dxgi_fallback(Some(Dxgi::Gptk), native, ARM, "", dxgi).as_deref(),
            Some("  dxgi = gptk not available on arm64, using wined3d")
        );
        let d3d9 = D3d9::effective(Some(D3d9::Mtld3d), native, ARM);
        assert_eq!(
            d3d9_fallback(Some(D3d9::Mtld3d), native, ARM, d3d9).as_deref(),
            Some("  d3d9 = mtld3d not available on aarch64, using wined3d")
        );
        // An x86 program in the same Wine keeps mtld3d without a word, as does
        // every program in the x86_64 Wine.
        for (arch, host) in [
            (Arch::X86_64, ARM),
            (Arch::I386, ARM),
            (Arch::X86_64, INTEL),
            (Arch::I386, INTEL),
        ] {
            let d3d9 = D3d9::effective(Some(D3d9::Mtld3d), arch, host);
            assert_eq!(d3d9, D3d9::Mtld3d);
            assert_eq!(d3d9_fallback(Some(D3d9::Mtld3d), arch, host, d3d9), None);
        }
    }

    #[test]
    fn the_log_labels_each_arch_for_its_host() {
        assert_eq!(arch_name(Arch::X86_64, INTEL), "x86_64");
        assert_eq!(arch_name(Arch::I386, INTEL), "i386");
        assert_eq!(arch_name(Arch::X86_64, ARM), "x86_64 (arm64ec)");
        assert_eq!(arch_name(Arch::I386, ARM), "i386");
        assert_eq!(arch_name(Arch::Aarch64, ARM), "aarch64");
    }

    #[test]
    fn the_builtin_rules_set_only_the_wildcard_env_for_a_game() {
        // Beyond the CEF helpers, which builtin.rs checks, a game gets env
        // from the built-in rules only in the arm64 Wine: the FEX and
        // KosmicKrisp entries every process gets. The exported function reads
        // WINE_COMPATDB, which a developer's shell may set; the answer is only
        // predictable without it.
        if std::env::var_os(ENV_VAR).is_some() {
            return;
        }
        let expected: &[u8] = match Host::current() {
            Host::X86_64 => b"",
            Host::Aarch64 => b"FEX_X87REDUCEDPRECISION=1\0MESA_KK_EXPERIMENTAL=dgc,xfb,sparse\0\0",
        };
        let path = c"/nonexistent/games/game.exe";
        let mut buf = [0xAA_u8; 128];
        // SAFETY: a valid NUL-terminated string and a buffer of its length.
        let ret = unsafe { compatdb_query_env(path.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        assert_eq!(usize::try_from(ret).ok(), Some(expected.len()));
        assert_eq!(buf.get(..expected.len()), Some(expected));
        assert!(
            buf.get(expected.len()..)
                .is_some_and(|rest| rest.iter().all(|&b| b == 0xAA))
        );
        // SAFETY: a valid NUL-terminated string and a null buffer of size 0.
        let ret = unsafe { compatdb_query_env(path.as_ptr(), std::ptr::null_mut(), 0) };
        assert_eq!(usize::try_from(ret).ok(), Some(expected.len()));
    }
}
