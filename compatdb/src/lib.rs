//! `compatdb.so`: the per-process settings library wine's ntdll dlopens from
//! `<ntdll_dir>/compatdb.so` in every process (the slot `CrossOver` hack 24067
//! provides), before any PE code runs. Its only entry point is a Mach-O
//! initializer.
//!
//! It is built for the arm64 Wine, which runs x86 programs under emulation
//! and native ARM64 ones.
//!
//! It holds the built-in rules, appends the ones passed in [`ENV_VAR`],
//! resolves the rules that match this process and applies them: the D3D9
//! and DXGI trees, the wined3d renderer, DLL load-order overrides, the DPI
//! awareness, large address awareness, whether the Mac driver reads raw
//! mouse input from `GameController`, and command-line/environment
//! rewrites. All the decision logic lives in [`compatdb_table`]; this crate
//! only moves bytes in and out of the running process, so its `unsafe` is
//! confined to a thin ntdll shell.
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
    ffi::{CStr, CString},
    fmt::Write as _,
};

use compatdb_table::{
    Arch, ArgumentsDropped, D3d9, DllOverrideReset, Dxgi, ENV_VAR, EnvStep, Origin, Resolution,
    Table, VersionInfo, Wined3d, basename_of, upstream,
};

use crate::ntdll::{Ntdll, Peb, Peb32, ProcessParameters, ProcessParameters32, TEB_PEB_OFFSET};

/// The Mach-O initializer, run from ntdll's `dlopen` in every wine process.
#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static INIT: extern "C" fn() = run;

/// The built-in rules followed by the rules from [`ENV_VAR`], plus what
/// could not be read from that variable, for the log.
fn load_table() -> (Table, Vec<String>) {
    // The built-in rules are compiled in; only the caller's rules travel in
    // the environment (absent unless something set it). They come after the
    // built-in ones, so within a tier they fold later.
    let mut table = builtin::table();
    let mut messages = Vec::new();
    if let Some(text) = read_env() {
        let (rules, diagnostics) = Table::parse(&text);
        messages.extend(diagnostics);
        table.rules.extend(rules.rules);
    }
    (table, messages)
}

extern "C" fn run() {
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
    let Some(process) = Process::current(&nt) else {
        log_warnings(&messages);
        return;
    };
    let mut resolution = table.resolve(&process.exe, &process.version);
    let arch = arch_name(process.arch);
    log::line(&format!(
        "{} [{arch}]{}",
        process.exe,
        fingerprint(&process.version)
    ));
    // What could not be read from WINE_COMPATDB goes in the block of the
    // process, next to the configuration it leaves.
    log_warnings(&messages);

    // Every process gets one tree of each family prepended: a rule's choice,
    // which for most is a built-in `*` rule's, or wined3d, upstream Wine's,
    // when no rule names one. Every tree serves every arch. Purely additive:
    // the default dirs hold nothing that could conflict, so nothing is ever
    // disabled.
    //
    // Every key gets a line, with where its value came from or, unset, what
    // upstream Wine does instead, so the block is the whole configuration of
    // the process.
    let pe_dir = process.arch.pe_dir();
    let dxgi = Dxgi::effective(resolution.dxgi, process.arch);
    apply::tree(
        &nt,
        pe_dir,
        "dxgi",
        dxgi.as_str(),
        &scalar_source(&resolution, "dxgi"),
    );
    let d3d9 = D3d9::effective(resolution.d3d9, process.arch);
    apply::tree(
        &nt,
        pe_dir,
        "d3d9",
        d3d9.as_str(),
        &scalar_source(&resolution, "d3d9"),
    );
    // One renderer for every wined3d user in the process, whichever tree or
    // API brought wined3d in. A renderer a rule sets travels as an env
    // entry, applied with the rest below, whose line shows it. Without one,
    // the log names the renderer the WINE_D3D_CONFIG the process ends up
    // with asks for: a rule's env, or else the one it inherited.
    let wined3d = resolution.apply_wined3d();
    let config = match resolution.d3d_config() {
        Some(value) => Some((value.to_owned(), resolution.d3d_config_origin())),
        None => compatdb_table::env_value(&process.env, Wined3d::CONFIG_VAR).map(|v| (v, None)),
    };
    log::line(&wined3d_line(
        wined3d,
        &scalar_source(&resolution, "wined3d"),
        resolution.reset_origin("wined3d"),
        config.as_ref().map(|(value, rule)| (value.as_str(), *rule)),
    ));

    log::line(&list_line(
        "dll_overrides",
        &resolution.dll_overrides,
        &resolution.dll_overrides_from,
        None,
    ));
    for reset in &resolution.dll_overrides_reset {
        log::line(&dll_reset_line(reset));
    }
    apply::dll_overrides(&nt, &resolution.dll_overrides);
    apply_hooks(&nt, &process, &resolution);
    let (cmd, appended) =
        compatdb_table::append_cmdline_each(&process.cmdline, &resolution.arguments);
    log::line(&list_line(
        "arguments",
        &resolution.arguments,
        &resolution.arguments_from,
        Some(&appended),
    ));
    for dropped in &resolution.arguments_dropped {
        log::line(&dropped_line(dropped));
    }
    // A default env entry leaves a variable the process already has alone,
    // whether it is in the environment block or only in the unix
    // environment.
    let has = |name: &str| {
        compatdb_table::env_value(&process.env, name).or_else(|| std::env::var(name).ok())
    };
    let steps = resolution.env_steps(|name| has(name).is_some());
    if steps.is_empty() {
        log::line(&unset_line("env"));
    }
    for step in &steps {
        log::line(&env_line(step, has(&step.name).as_deref()));
    }
    let env: Vec<(String, String)> = steps
        .iter()
        .filter(|step| step.applied)
        .map(|step| (step.name.clone(), step.value.clone()))
        .collect();
    // The env goes into both the environment block, for the PE side and
    // what reads it at process init (FEX, wined3d), and the unix
    // environment, for unix-side readers (winemetal, mtld3d, Metal,
    // KosmicKrisp).
    apply::rewrite_params(&nt, &process, cmd.as_deref(), &env);
    apply::unix_env(&env);
}

/// Hand the settings ntdll stores for Wine to read later to their hooks, each
/// with its log line: the DPI awareness, large address awareness, the
/// `GameController` mouse setting, whether no-execute stays on and which
/// OpenGL the Mac driver gives.
fn apply_hooks(nt: &Ntdll, process: &Process, resolution: &Resolution) {
    match resolution.dpi_aware {
        Some(aware) => apply::dpi_awareness(nt, aware, &scalar_source(resolution, "dpi_aware")),
        None => log::line(&scalar_unset_line(resolution, "dpi_aware")),
    }
    match resolution.large_address_aware {
        Some(laa) => apply::large_address_aware(
            nt,
            laa,
            &scalar_source(resolution, "large_address_aware"),
            process.params32.is_some(),
        ),
        None => log::line(&scalar_unset_line(resolution, "large_address_aware")),
    }
    match resolution.gcmouse {
        Some(on) => apply::gcmouse(nt, on, &scalar_source(resolution, "gcmouse")),
        None => log::line(&scalar_unset_line(resolution, "gcmouse")),
    }
    match resolution.force_nx {
        Some(on) => apply::force_nx(
            nt,
            on,
            &scalar_source(resolution, "force_nx"),
            process.params32.is_some(),
        ),
        None => log::line(&scalar_unset_line(resolution, "force_nx")),
    }
    match resolution.opengl {
        Some(opengl) => apply::opengl(nt, opengl, &scalar_source(resolution, "opengl")),
        None => log::line(&scalar_unset_line(resolution, "opengl")),
    }
}

/// Where the setting `key` came from, for the log: its rule's source (see
/// [`Origin::label`]), or, when no rule set it, what upstream Wine does,
/// followed by the rule that reset it to `default`, when one did.
fn source(key: &str, origin: Option<&Origin>, reset: Option<&Origin>) -> String {
    match (origin, reset) {
        (Some(origin), _) => origin.label(),
        (None, Some(reset)) => format!(
            "unset (upstream: {}), reset by {}",
            upstream(key),
            reset.label()
        ),
        (None, None) => format!("unset (upstream: {})", upstream(key)),
    }
}

/// [`source`] of the scalar setting `key` of `resolution`.
fn scalar_source(resolution: &Resolution, key: &str) -> String {
    source(key, resolution.origin(key), resolution.reset_origin(key))
}

/// The log line of the scalar setting `key` of `resolution`, which no rule
/// set or the last rule for it reset.
fn scalar_unset_line(resolution: &Resolution, key: &str) -> String {
    format!("  {key} = {}", scalar_source(resolution, key))
}

/// The log line of the setting `key` that no rule set.
fn unset_line(key: &str) -> String {
    format!("  {key} = {}", source(key, None, None))
}

/// The log line of a `<module>=default` entry of `dll_overrides`: where it
/// came from, and whose overrides of the module it removed.
fn dll_reset_line(reset: &DllOverrideReset) -> String {
    let what = if reset.removed.is_empty() {
        String::from("no override to remove")
    } else {
        let from: Vec<String> = reset.removed.iter().map(Origin::label).collect();
        format!("removes the override from {}", from.join("; "))
    };
    format!(
        "  dll_overrides {}=default, {}: {what}",
        reset.module,
        reset.by.label()
    )
}

/// The log line of a list setting: each entry with where it came from, and,
/// when `applied` is given, whether it went in.
fn list_line(key: &str, entries: &[String], from: &[Origin], applied: Option<&[bool]>) -> String {
    if entries.is_empty() {
        return unset_line(key);
    }
    let items: Vec<String> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let label = from.get(i).map(Origin::label).unwrap_or_default();
            match applied.map(|applied| applied.get(i).copied().unwrap_or(false)) {
                Some(true) => format!("{entry} ({label}, appended)"),
                Some(false) => format!("{entry} ({label}, already present)"),
                None => format!("{entry} ({label})"),
            }
        })
        .collect();
    format!("  {key} = {}", items.join("; "))
}

/// The log line of an `arguments_override` that dropped arguments: where it
/// came from, and each argument it dropped with where that came from.
fn dropped_line(dropped: &ArgumentsDropped) -> String {
    let items: Vec<String> = dropped
        .entries
        .iter()
        .map(|(entry, from)| format!("{entry} ({})", from.label()))
        .collect();
    format!(
        "  arguments_override, {}: dropped {}",
        dropped.by.label(),
        items.join("; ")
    )
}

/// The log line of an env entry: its value, where it came from and what it
/// did. `has` is the value the process had for the variable, if any.
fn env_line(step: &EnvStep, has: Option<&str>) -> String {
    let what = match (&step.replaced_by, step.applied, step.value.is_empty()) {
        (Some(later), _, _) => format!("replaced by a later entry from {}", later.label()),
        (None, false, _) => format!(
            "left alone, the process has {}={}",
            step.name,
            has.unwrap_or_default()
        ),
        (None, true, true) => match (&step.removes, has) {
            (Some(earlier), _) => format!("removes the value from {}", earlier.label()),
            (None, Some(_)) => String::from("removed"),
            (None, None) => String::from("not set, nothing to remove"),
        },
        (None, true, false) => String::from("set"),
    };
    format!(
        "  env {}={}, {}: {what}",
        step.name,
        step.value,
        step.origin.label()
    )
}

/// Log each message about the rules from [`ENV_VAR`] as a warning: a field
/// or a line that was ignored.
fn log_warnings(messages: &[String]) {
    for m in messages {
        log::line(&format!("  warning: {m}"));
    }
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

/// The log line naming the wined3d renderer of a process and where it comes
/// from, like the other settings. `set` is the renderer a rule set and
/// `source` the [`source`] of the setting; `reset` is the rule that reset it
/// to `default`, if any; `config` is the `WINE_D3D_CONFIG` the process has
/// otherwise, with the rule whose `env` set it, or `None` for one it
/// inherited. Without either, the line says what upstream Wine does.
fn wined3d_line(
    set: Option<Wined3d>,
    source: &str,
    reset: Option<&Origin>,
    config: Option<(&str, Option<&Origin>)>,
) -> String {
    let asks = config.and_then(|(value, from)| Wined3d::renderer_in(value).map(|r| (r, from)));
    let reset = reset.map_or_else(String::new, |by| format!(", reset by {}", by.label()));
    match (set, asks) {
        (Some(wined3d), _) => format!("  wined3d = {}, {source}", wined3d.as_str()),
        (None, Some((renderer, None))) => format!(
            "  wined3d = {renderer}, from the inherited {}{reset}",
            Wined3d::CONFIG_VAR
        ),
        (None, Some((renderer, Some(from)))) => format!(
            "  wined3d = {renderer}, from the {} of {}{reset}",
            Wined3d::CONFIG_VAR,
            from.label()
        ),
        (None, None) => format!("  wined3d = {source}"),
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
    use super::*;

    #[test]
    fn a_native_arm64_program_gets_dxmt_and_mtld3d_and_either_tree_a_rule_asks_for() {
        let native = Arch::Aarch64;
        // The built-in rules give it dxmt and mtld3d, as an x86 program.
        let game = builtin::table().resolve("game.exe", &VersionInfo::default());
        assert_eq!(Dxgi::effective(game.dxgi, native), Dxgi::Dxmt);
        assert_eq!(D3d9::effective(game.d3d9, native), D3d9::Mtld3d);
        // Without a rule, wined3d, as in upstream Wine.
        assert_eq!(Dxgi::effective(None, native), Dxgi::Wined3d);
        assert_eq!(D3d9::effective(None, native), D3d9::Wined3d);
        // Either tree of each family is what it gets when a rule asks for it.
        for requested in [Dxgi::Dxmt, Dxgi::Wined3d] {
            assert_eq!(Dxgi::effective(Some(requested), native), requested);
        }
        for requested in [D3d9::Mtld3d, D3d9::Wined3d] {
            assert_eq!(D3d9::effective(Some(requested), native), requested);
        }
    }

    /// The source of the built-in rule for World of Warcraft, as the log
    /// names it.
    fn wow() -> Origin {
        Origin::BuiltInFor("WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)".into())
    }

    #[test]
    fn the_wined3d_line_names_the_renderer() {
        let unset = source("wined3d", None, None);
        let line = Origin::Line(2);
        // A renderer a rule set wins the line over any config, and reads like
        // any other setting.
        for config in [None, Some(("renderer=vulkan", None))] {
            assert_eq!(
                wined3d_line(Some(Wined3d::Gl), &line.label(), None, config),
                "  wined3d = gl, WINE_COMPATDB line 2"
            );
        }
        assert_eq!(
            wined3d_line(Some(Wined3d::Vulkan), &wow().label(), None, None),
            "  wined3d = vulkan, built-in for WoW.exe (Blizzard Entertainment, World of \
             Warcraft, WoW.exe)"
        );
        // Not set: what a config asks for, inherited or from a rule's env,
        // else what upstream Wine does, which may still come from the
        // registry.
        assert_eq!(
            wined3d_line(None, &unset, None, Some(("csmt=0,renderer=vulkan", None))),
            "  wined3d = vulkan, from the inherited WINE_D3D_CONFIG"
        );
        assert_eq!(
            wined3d_line(None, &unset, None, Some(("renderer=vulkan", Some(&line)))),
            "  wined3d = vulkan, from the WINE_D3D_CONFIG of WINE_COMPATDB line 2"
        );
        assert_eq!(
            wined3d_line(
                None,
                &unset,
                None,
                Some(("renderer=vulkan", Some(&Origin::BuiltIn)))
            ),
            "  wined3d = vulkan, from the WINE_D3D_CONFIG of built-in"
        );
        for config in [None, Some(("", None)), Some(("csmt=0", Some(&line)))] {
            assert_eq!(
                wined3d_line(None, &unset, None, config),
                format!("  wined3d = {unset}")
            );
        }
        // A reset says so, with or without a config that asks for a
        // renderer.
        let reset = source("wined3d", None, Some(&line));
        assert_eq!(
            wined3d_line(None, &reset, Some(&line), None),
            format!(
                "  wined3d = unset (upstream: {}), reset by WINE_COMPATDB line 2",
                upstream("wined3d")
            )
        );
        assert_eq!(
            wined3d_line(None, &reset, Some(&line), Some(("renderer=vulkan", None))),
            "  wined3d = vulkan, from the inherited WINE_D3D_CONFIG, reset by WINE_COMPATDB line 2"
        );
        assert_eq!(
            wined3d_line(
                None,
                &reset,
                Some(&line),
                Some(("renderer=gl", Some(&Origin::Line(3))))
            ),
            "  wined3d = gl, from the WINE_D3D_CONFIG of WINE_COMPATDB line 3, reset by \
             WINE_COMPATDB line 2"
        );
    }

    #[test]
    fn a_reset_scalar_and_a_default_dll_override_say_which_line_reset_them() {
        assert_eq!(
            source("dpi_aware", None, Some(&Origin::Line(3))),
            format!(
                "unset (upstream: {}), reset by WINE_COMPATDB line 3",
                upstream("dpi_aware")
            )
        );
        // A value set after the reset is what the line shows.
        assert_eq!(
            source("dpi_aware", Some(&Origin::Line(4)), None),
            "WINE_COMPATDB line 4"
        );
        let reset = |removed: Vec<Origin>| DllOverrideReset {
            module: "mscoree".into(),
            by: Origin::Line(1),
            removed,
        };
        assert_eq!(
            dll_reset_line(&reset(vec![Origin::BuiltIn])),
            "  dll_overrides mscoree=default, WINE_COMPATDB line 1: removes the override from \
             built-in"
        );
        assert_eq!(
            dll_reset_line(&reset(vec![Origin::BuiltIn, wow()])),
            "  dll_overrides mscoree=default, WINE_COMPATDB line 1: removes the override from \
             built-in; built-in for WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)"
        );
        assert_eq!(
            dll_reset_line(&reset(Vec::new())),
            "  dll_overrides mscoree=default, WINE_COMPATDB line 1: no override to remove"
        );
    }

    #[test]
    fn the_list_and_env_lines_name_the_source_of_each_entry_and_what_happened() {
        assert_eq!(
            list_line("arguments", &[], &[], Some(&[])),
            "  arguments = unset (upstream: the command line the process was started with)"
        );
        let args = ["--in-process-gpu".to_string(), "-x".to_string()];
        let from = [wow(), Origin::Line(1)];
        assert_eq!(
            list_line("arguments", &args, &from, Some(&[false, true])),
            "  arguments = --in-process-gpu (built-in for WoW.exe (Blizzard Entertainment, \
             World of Warcraft, WoW.exe), already present); -x (WINE_COMPATDB line 1, \
             appended)"
        );
        assert_eq!(
            list_line(
                "dll_overrides",
                &["mscoree,mshtml=".into()],
                &[Origin::BuiltIn],
                None
            ),
            "  dll_overrides = mscoree,mshtml= (built-in)"
        );
        let step = |value: &str, applied, replaced_by: Option<Origin>| EnvStep {
            name: "FEX_X87REDUCEDPRECISION".into(),
            value: value.into(),
            origin: Origin::BuiltIn,
            applied,
            replaced_by,
            removes: None,
        };
        assert_eq!(
            env_line(&step("1", true, None), None),
            "  env FEX_X87REDUCEDPRECISION=1, built-in: set"
        );
        assert_eq!(
            env_line(&step("", true, None), Some("0")),
            "  env FEX_X87REDUCEDPRECISION=, built-in: removed"
        );
        assert_eq!(
            env_line(&step("", true, None), None),
            "  env FEX_X87REDUCEDPRECISION=, built-in: not set, nothing to remove"
        );
        // A removal after an earlier entry set the variable undoes that one,
        // whether or not the process had it.
        let undo = EnvStep {
            origin: Origin::Line(3),
            removes: Some(Origin::BuiltIn),
            ..step("", true, None)
        };
        assert_eq!(
            env_line(&undo, None),
            "  env FEX_X87REDUCEDPRECISION=, WINE_COMPATDB line 3: removes the value from \
             built-in"
        );
        assert_eq!(
            env_line(&step("1", false, None), Some("0")),
            "  env FEX_X87REDUCEDPRECISION=1, built-in: left alone, the process has \
             FEX_X87REDUCEDPRECISION=0"
        );
        assert_eq!(
            env_line(&step("1", true, Some(Origin::Line(2))), None),
            "  env FEX_X87REDUCEDPRECISION=1, built-in: replaced by a later entry from \
             WINE_COMPATDB line 2"
        );
        assert_eq!(
            env_line(&step("1", true, Some(wow())), None),
            "  env FEX_X87REDUCEDPRECISION=1, built-in: replaced by a later entry from \
             built-in for WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)"
        );
    }

    #[test]
    fn an_arguments_override_line_names_what_it_dropped() {
        let dropped = ArgumentsDropped {
            by: Origin::Line(1),
            entries: vec![
                ("--in-process-gpu".into(), wow()),
                ("-x".into(), Origin::BuiltIn),
            ],
        };
        assert_eq!(
            dropped_line(&dropped),
            "  arguments_override, WINE_COMPATDB line 1: dropped --in-process-gpu (built-in \
             for WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)); -x (built-in)"
        );
    }

    #[test]
    fn an_unset_line_says_what_upstream_wine_does() {
        assert_eq!(
            unset_line("gcmouse"),
            "  gcmouse = unset (upstream: off, the cursor's movement)"
        );
        assert_eq!(
            unset_line("force_nx"),
            "  force_nx = unset (upstream: no-execute off when the executable or a DLL lacks the \
             NX_COMPAT flag)"
        );
        assert_eq!(
            unset_line("opengl"),
            "  opengl = unset (upstream: Apple's OpenGL)"
        );
        assert_eq!(source("dxgi", None, None), "unset (upstream: wined3d)");
        assert_eq!(source("dxgi", Some(&Origin::BuiltIn), None), "built-in");
        assert_eq!(
            source("dxgi", Some(&Origin::Line(4)), None),
            "WINE_COMPATDB line 4"
        );
        for (key, _) in compatdb_table::UPSTREAM {
            assert!(!unset_line(key).contains("unknown"), "{key}");
        }
    }

    #[test]
    fn the_log_labels_each_arch() {
        assert_eq!(arch_name(Arch::X86_64), "x86_64 (arm64ec)");
        assert_eq!(arch_name(Arch::I386), "i386");
        assert_eq!(arch_name(Arch::Aarch64), "aarch64");
    }

    #[test]
    fn without_wine_compatdb_the_table_is_the_builtin_one() {
        // load_table reads WINE_COMPATDB, which a developer's shell may set;
        // the answer is only predictable without it.
        if std::env::var_os(ENV_VAR).is_some() {
            return;
        }
        let (table, messages) = load_table();
        assert_eq!(table, builtin::table());
        assert_eq!(messages, [] as [String; 0]);
    }
}
