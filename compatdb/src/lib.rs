//! `compatdb.so`: the per-process settings library wine's ntdll dlopens from
//! `<ntdll_dir>/compatdb.so` in every process (the slot `CrossOver` hack 24067
//! provides), before any PE code runs. Its only entry point is a Mach-O
//! initializer.
//!
//! It is built for the arm64 Wine, which runs x86 programs under emulation
//! and native ARM64 ones.
//!
//! It holds the built-in rules, overlays the ones passed in
//! [`ENV_VAR`](compatdb_table::ENV_VAR), resolves the rules that match this
//! process and applies them: the D3D9 and DXGI trees, the wined3d renderer,
//! DLL load-order overrides, the DPI awareness, large address awareness,
//! whether the Mac driver reads raw mouse input from `GameController`, and
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
    ffi::{CStr, CString},
    fmt::Write as _,
};

use compatdb_table::{Arch, D3d9, Dxgi, ENV_VAR, Table, VersionInfo, Wined3d, basename_of};

use crate::ntdll::{Ntdll, Peb, Peb32, ProcessParameters, ProcessParameters32, TEB_PEB_OFFSET};

/// The Mach-O initializer, run from ntdll's `dlopen` in every wine process.
#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static INIT: extern "C" fn() = run;

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
    if resolution.matched.is_empty() {
        // Diagnose a miss by the fingerprint on the line above: an empty one
        // means the image carries no version resource for a rule to pin.
        log::line("  no rule matched");
    } else {
        log::line(&format!("  rules = {}", resolution.matched.join(", ")));
    }

    // Every process gets one tree of each family prepended: a rule's choice,
    // or the arch default. Every tree serves every arch. Purely additive: the
    // default dirs hold nothing that could conflict, so nothing is ever
    // disabled.
    let pe_dir = process.arch.pe_dir();
    let dxgi = Dxgi::effective(resolution.dxgi, process.arch);
    apply::tree(&nt, pe_dir, "dxgi", dxgi.as_str());
    let d3d9 = D3d9::effective(resolution.d3d9, process.arch);
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
    if let Some(gcmouse) = resolution.gcmouse {
        apply::gcmouse(&nt, gcmouse);
    }
    if !resolution.arguments.is_empty() {
        log::line(&format!("  arguments = {}", resolution.arguments.join(" ")));
    }
    // A default env entry leaves a variable the process already has alone,
    // whether it is in the environment block or only in the unix
    // environment.
    let steps = resolution.env_steps(|name| {
        compatdb_table::env_value(&process.env, name).is_some() || std::env::var_os(name).is_some()
    });
    let env: Vec<(String, String)> = steps
        .iter()
        .filter(|step| step.applied)
        .map(|step| (step.name.clone(), step.value.clone()))
        .collect();
    if !env.is_empty() {
        let list: Vec<String> = env
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        log::line(&format!("  env = {}", list.join(", ")));
    }
    for step in steps.iter().filter(|step| !step.applied) {
        log::line(&format!(
            "  env {}={} (rule {}) left alone: the process has {}",
            step.name, step.value, step.rule, step.name
        ));
    }
    // The env goes into both the environment block, for the PE side and
    // what reads it at process init (FEX, wined3d), and the unix
    // environment, for unix-side readers (winemetal, mtld3d, Metal,
    // KosmicKrisp).
    apply::rewrite_params(&nt, &process, &resolution.arguments, &env);
    apply::unix_env(&env);
}

/// Log each message about the overrides from [`ENV_VAR`] as a warning: a
/// field or record that was ignored, or an override that names no rule.
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
    use super::*;

    #[test]
    fn a_native_arm64_program_gets_dxmt_and_mtld3d_and_either_tree_a_rule_asks_for() {
        let native = Arch::Aarch64;
        // The defaults are dxmt and mtld3d, as for an x86 program.
        assert_eq!(Dxgi::effective(None, native), Dxgi::Dxmt);
        assert_eq!(D3d9::effective(None, native), D3d9::Mtld3d);
        // Either tree of each family is what it gets when a rule asks for it.
        for requested in [Dxgi::Dxmt, Dxgi::Wined3d] {
            assert_eq!(Dxgi::effective(Some(requested), native), requested);
        }
        for requested in [D3d9::Mtld3d, D3d9::Wined3d] {
            assert_eq!(D3d9::effective(Some(requested), native), requested);
        }
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
