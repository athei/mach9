//! `compatdb.so`: the per-process settings library wine's ntdll dlopens from
//! `<ntdll_dir>/compatdb.so` in every process (the slot `CrossOver` hack 24067
//! provides), before any PE code runs. Its main entry point is a Mach-O
//! initializer; an ntdll that asks can also call [`compatdb_query_env`] for
//! the rule environment to set before it execs a process.
//!
//! It is built for the arm64 Wine, which runs x86 programs under emulation
//! and native ARM64 ones.
//!
//! It holds the built-in rules, overlays the ones passed in
//! [`ENV_VAR`](compatdb_table::ENV_VAR), resolves the rules that match this
//! process and applies them: the D3D9 and DXGI trees, the wined3d renderer,
//! DLL load-order overrides, the DPI awareness, large address awareness, and
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
mod ntdll;
mod version;

use std::{
    ffi::{CStr, CString, OsStr, c_char, c_int},
    fmt::Write as _,
    os::unix::ffi::OsStrExt as _,
    path::Path,
};

use compatdb_table::{
    Arch, D3d9, Dxgi, ENV_VAR, Resolution, Table, VersionInfo, Wined3d, basename_of,
};

use crate::ntdll::{Ntdll, Peb, Peb32, ProcessParameters, ProcessParameters32, TEB_PEB_OFFSET};

/// The Mach-O initializer, run from ntdll's `dlopen` in every wine process.
#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static INIT: extern "C" fn() = init;

extern "C" fn init() {
    // ntdll can load the library before it is initialized itself, only to
    // call compatdb_query_env, and says so through compatdb_preinit_query.
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

/// The rule environment of the executable at `path`, for ntdll to set in the
/// unix environment right before it execs the process.
///
/// The process gets its environment block and unix environment from compatdb
/// only once it runs, which is too late for whatever starts ahead of it.
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
/// ntdll may call this before it is initialized itself, so it reads only the
/// rule table and the file: no PEB, no TEB and no ntdll symbol. It keeps no
/// state between calls. The path is a unix path when ntdll could resolve one;
/// otherwise it may be a DOS path, in which case only the basename is known
/// and the version fingerprint is empty.
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
    let mut resolution = table.resolve(&exe, &version);
    // The env the process gets includes what its wined3d renderer needs.
    resolution.apply_wined3d();
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
    let Some(process) = Process::current(&nt) else {
        return;
    };
    let mut resolution = table.resolve(&process.exe, &process.version);
    let arch = arch_name(process.arch);
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
    // or the arch default, and wined3d where the one asked for cannot load
    // (dxmt or mtld3d for a native ARM64 program). Purely additive: the
    // default dirs hold nothing that could conflict, so nothing is ever
    // disabled.
    let pe_dir = process.arch.pe_dir();
    let dxgi = Dxgi::effective(resolution.dxgi, process.arch);
    if let Some(note) = fallback(
        "dxgi",
        resolution.dxgi.map(Dxgi::as_str),
        dxgi.as_str(),
        process.arch,
    ) {
        log::line(&note);
    }
    apply::tree(&nt, pe_dir, "dxgi", dxgi.as_str());
    let d3d9 = D3d9::effective(resolution.d3d9, process.arch);
    if let Some(note) = fallback(
        "d3d9",
        resolution.d3d9.map(D3d9::as_str),
        d3d9.as_str(),
        process.arch,
    ) {
        log::line(&note);
    }
    apply::tree(&nt, pe_dir, "d3d9", d3d9.as_str());
    // One renderer for every wined3d user in the process, whichever tree or
    // API brought wined3d in. A renderer a rule sets travels as an env
    // entry, applied with the rest below. Without one, the log names the
    // renderer the WINE_D3D_CONFIG the process ends up with asks for: a
    // rule's env, or else the one it inherited.
    let wined3d = resolution.apply_wined3d();
    let config = match resolution.d3d_config() {
        Some(value) => Some(value.to_owned()),
        None => compatdb_table::env_value(&process.env, Wined3d::CONFIG_VAR),
    };
    log::line(&wined3d_line(wined3d, config.as_deref()));

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

/// The arch of a process as the log prints it. An `x86_64` program runs in an
/// ARM64EC process, which the label says.
const fn arch_name(arch: Arch) -> &'static str {
    match arch {
        Arch::X86_64 => "x86_64 (arm64ec)",
        Arch::I386 => "i386",
        Arch::Aarch64 => "aarch64",
    }
}

/// The log line for a process whose rule asks `family` (`dxgi` or `d3d9`) for
/// an implementation that cannot load for its arch (dxmt or mtld3d for a
/// native ARM64 program). `got` is what the family's `effective` chose.
/// `None` when the process gets what it asked for, or asked for nothing: the
/// default always loads.
fn fallback(family: &str, requested: Option<&str>, got: &str, arch: Arch) -> Option<String> {
    let want = requested?;
    (want != got).then(|| {
        format!(
            "  {family} = {want} not available on {}, using {got}",
            arch_name(arch)
        )
    })
}

/// The log line naming the wined3d renderer of a process and where it comes
/// from, in the style of the tree lines. `set` is the renderer a rule set,
/// `config` the `WINE_D3D_CONFIG` the process has
/// otherwise: from a rule's `env`, or inherited. Without either, the line says
/// `default`: wined3d may still take a renderer from its registry settings,
/// and uses OpenGL without one.
fn wined3d_line(set: Option<Wined3d>, config: Option<&str>) -> String {
    match (set, config.and_then(Wined3d::renderer_in)) {
        (Some(wined3d), _) => format!(
            "  wined3d = {} (through {})",
            wined3d.as_str(),
            Wined3d::CONFIG_VAR
        ),
        (None, Some(renderer)) => format!(
            "  wined3d = {renderer} (not set; {} asks for it)",
            Wined3d::CONFIG_VAR
        ),
        (None, None) => String::from("  wined3d = default (not set)"),
    }
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
    /// The current process.
    fn current(nt: &Ntdll) -> Option<Self> {
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
        // 0 otherwise. Wine runs only x86 programs under wow64, with an
        // emulator, so such a process is i386. A 64-bit process is native
        // from Wine's side either way, ARM64EC for an x86_64 program, so the
        // main image's machine tells an x86_64 program from a native ARM64
        // one; see Arch::of_process. An x86_64 program stays Arch::X86_64, for
        // the log and the Direct3D defaults; the rules themselves match on
        // the image name and version alone.
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
        let arch = Arch::of_process(params32.is_some(), machine);

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

    /// The fallback line for each family, as `run` asks for it.
    fn dxgi_line(requested: Option<Dxgi>, arch: Arch) -> Option<String> {
        let dxgi = Dxgi::effective(requested, arch);
        fallback("dxgi", requested.map(Dxgi::as_str), dxgi.as_str(), arch)
    }

    fn d3d9_line(requested: Option<D3d9>, arch: Arch) -> Option<String> {
        let d3d9 = D3d9::effective(requested, arch);
        fallback("d3d9", requested.map(D3d9::as_str), d3d9.as_str(), arch)
    }

    #[test]
    fn an_x86_program_gets_what_it_asks_for_without_a_word() {
        for arch in [Arch::X86_64, Arch::I386] {
            for requested in [None, Some(Dxgi::Dxmt), Some(Dxgi::Wined3d)] {
                assert_eq!(dxgi_line(requested, arch), None, "{arch:?} {requested:?}");
            }
            for requested in [None, Some(D3d9::Mtld3d), Some(D3d9::Wined3d)] {
                assert_eq!(d3d9_line(requested, arch), None, "{arch:?} {requested:?}");
            }
        }
    }

    #[test]
    fn a_native_arm64_program_gets_wined3d_quietly_and_rules_say_why() {
        let native = Arch::Aarch64;
        // The default is wined3d for both families, with nothing to log.
        assert_eq!(Dxgi::effective(None, native), Dxgi::Wined3d);
        assert_eq!(dxgi_line(None, native), None);
        assert_eq!(D3d9::effective(None, native), D3d9::Wined3d);
        assert_eq!(d3d9_line(None, native), None);
        assert_eq!(dxgi_line(Some(Dxgi::Wined3d), native), None);
        assert_eq!(d3d9_line(Some(D3d9::Wined3d), native), None);
        // A rule for dxmt or mtld3d falls back, and the line names the arch.
        assert_eq!(
            dxgi_line(Some(Dxgi::Dxmt), native).as_deref(),
            Some("  dxgi = dxmt not available on aarch64, using wined3d")
        );
        assert_eq!(
            d3d9_line(Some(D3d9::Mtld3d), native).as_deref(),
            Some("  d3d9 = mtld3d not available on aarch64, using wined3d")
        );
    }

    #[test]
    fn the_wined3d_line_names_the_renderer() {
        // A renderer a rule set wins the line over any config.
        for config in [None, Some("renderer=vulkan")] {
            assert_eq!(
                wined3d_line(Some(Wined3d::Gl), config),
                "  wined3d = gl (through WINE_D3D_CONFIG)"
            );
        }
        assert_eq!(
            wined3d_line(Some(Wined3d::Vulkan), None),
            "  wined3d = vulkan (through WINE_D3D_CONFIG)"
        );
        // Not set: what an inherited or env-rule config asks for, else
        // wined3d's default, which may still come from the registry.
        assert_eq!(
            wined3d_line(None, Some("csmt=0,renderer=vulkan")),
            "  wined3d = vulkan (not set; WINE_D3D_CONFIG asks for it)"
        );
        for config in [None, Some(""), Some("csmt=0")] {
            assert_eq!(wined3d_line(None, config), "  wined3d = default (not set)");
        }
    }

    #[test]
    fn the_env_query_carries_the_wined3d_renderer_a_rule_sets() -> TestResult {
        let table = Table {
            rules: vec![Rule {
                name: "game".into(),
                exe: "Game.exe".into(),
                wined3d: Some(Wined3d::Vulkan),
                env: vec![pair("WINE_D3D_CONFIG", "csmt=0")],
                ..Rule::default()
            }],
        };
        let (_, _, env) = query_env(&table, b"/nonexistent/games/game.exe").ok_or("game")?;
        assert_eq!(
            env,
            [
                pair("WINE_D3D_CONFIG", "csmt=0"),
                pair("WINE_D3D_CONFIG", "renderer=vulkan,csmt=0"),
            ]
        );
        Ok(())
    }

    #[test]
    fn the_log_labels_each_arch() {
        assert_eq!(arch_name(Arch::X86_64), "x86_64 (arm64ec)");
        assert_eq!(arch_name(Arch::I386), "i386");
        assert_eq!(arch_name(Arch::Aarch64), "aarch64");
    }

    #[test]
    fn the_builtin_rules_set_only_the_wildcard_env_for_a_game() {
        // Beyond the CEF helpers, which builtin.rs checks, a game gets env
        // from the built-in rules only from the FEX and KosmicKrisp entries
        // every process gets. The exported function reads WINE_COMPATDB,
        // which a developer's shell may set; the answer is only predictable
        // without it.
        if std::env::var_os(ENV_VAR).is_some() {
            return;
        }
        let expected: &[u8] = b"FEX_X87REDUCEDPRECISION=1\0MESA_KK_EXPERIMENTAL=dgc,xfb,sparse\0\0";
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
