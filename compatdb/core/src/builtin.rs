//! The built-in rules, compiled into `compatdb.so`.
//!
//! This is static data the library owns, so it never travels through the
//! environment. Only the caller's rules are serialized into `WINE_COMPATDB`;
//! the library appends them to these, and every rule that matches a process
//! applies.
//!
//! Every rule that names an executable also carries a version fingerprint, so
//! none of them can catch an unrelated program that happens to share a file
//! name. The exceptions are the [`defaults`], which match every process on
//! purpose: each is a default that a built-in rule for one program, or any
//! `WINE_COMPATDB` rule that sets the same key, overrides. A rule for one
//! program folds after a `*` rule from `WINE_COMPATDB`, so its fix survives
//! launch-wide settings; a `WINE_COMPATDB` rule for that program changes it
//! (see [`Table::resolve`]).

use std::fmt;

#[cfg(doc)]
use crate::Table;
use crate::{
    Argument, D3d9, DllOverride, Dxgi, EnvEntry, Exe, Fingerprint, Key, OpenGl, Rule, Setting,
};

/// The rules the bundle ships with: the [`defaults`] for every process, then
/// the rules for one program each ([`programs`]). [`Table::builtin`] is the
/// table of them.
#[must_use]
pub fn rules() -> Vec<Rule> {
    defaults().into_iter().chain(programs()).collect()
}

/// The rules that match every process, each turning on one behaviour that
/// differs from upstream Wine.
#[must_use]
pub fn defaults() -> Vec<Rule> {
    vec![
        dxgi_dxmt(),
        d3d9_mtld3d(),
        dpi_aware(),
        gcmouse(),
        fex_reduced_x87_precision(),
        opengl_egl(),
        force_nx(),
        no_mono_or_gecko(),
    ]
}

/// The rules for one program each, which beat the [`defaults`] for it.
#[must_use]
pub fn programs() -> Vec<Rule> {
    vec![
        rockstar_games_launcher(),
        social_club_helper(),
        steam_web_helper(),
        gta_iv(),
        nostale(),
        age_of_empires_ii_hd(),
        world_of_warcraft(),
    ]
}

/// DXMT for D3D10, D3D11 and D3D12 in every process.
///
/// D3D10, D3D11 and D3D12 go through DXMT, which translates them to Metal
/// directly, instead of through wined3d, upstream Wine's implementation,
/// which draws through OpenGL or Vulkan and reaches a lower feature level and
/// frame rate on a Mac. D3D12 is vkd3d-proton in the same tree, which presents
/// through DXMT's dxgi.dll; the wined3d tree has no D3D12 at all. A rule for
/// one program with `dxgi = wined3d` gives it wined3d, as the Rockstar
/// launcher's does, and a `*` rule with it gives wined3d to every process
/// that no built-in rule for one program pins to a tree.
#[must_use]
pub fn dxgi_dxmt() -> Rule {
    Rule {
        exe: Exe::Any,
        dxgi: Setting::Set(Dxgi::Dxmt),
        ..Rule::default()
    }
}

/// mtld3d for D3D9 in every process.
///
/// D3D9 goes through mtld3d, which translates it to Metal directly, instead
/// of through wined3d, for the same reason as [`dxgi_dxmt`]. A rule for one
/// program with `d3d9 = wined3d` gives it wined3d, and a `*` rule with it
/// gives every process wined3d.
#[must_use]
pub fn d3d9_mtld3d() -> Rule {
    Rule {
        exe: Exe::Any,
        d3d9: Setting::Set(D3d9::Mtld3d),
        ..Rule::default()
    }
}

/// Every process is DPI-aware.
///
/// Programs under this Wine are treated as DPI-aware unless a rule says
/// otherwise. An unaware program on a scaled desktop gets its window and
/// mouse coordinates scaled while display modes are not, so a game that sizes
/// itself from the mode list draws and reads the pointer in two different
/// coordinate spaces.
#[must_use]
pub fn dpi_aware() -> Rule {
    Rule {
        exe: Exe::Any,
        dpi_aware: Setting::Set(true),
        ..Rule::default()
    }
}

/// Mouse movement from GameController in every process.
///
/// The Mac driver takes the mouse movement of raw input (`WM_INPUT`) and
/// DirectInput from GameController's `GCMouse`, which reports the device's
/// own counts. Otherwise that movement is the cursor's, which macOS has
/// already accelerated and, on a retina display, scaled, so mouselook in a
/// game that reads raw input or DirectInput turns further the faster the
/// hand moves. winemac.drv reads the value through ntdll when it reads its
/// settings, and without one keeps upstream Wine's behaviour, the cursor's
/// movement.
///
/// It matches every process, like the other [`defaults`]. A rule for one
/// game with `gcmouse = false` gives it the cursor's movement again: for a
/// game whose sensitivity was tuned to that, a mouse that GameController
/// reports twice, or a regression. A `*` rule with it does the same for every
/// process.
#[must_use]
pub fn gcmouse() -> Rule {
    Rule {
        exe: Exe::Any,
        gcmouse: Setting::Set(true),
        ..Rule::default()
    }
}

/// FEX's reduced x87 precision in every process.
///
/// x86 code runs under FEX, which by default emulates x87 floating-point
/// code, which games built for 32-bit x86 use heavily, at its full 80-bit
/// precision, and such games run slowly that way. Its `X87ReducedPrecision`
/// option computes with 64-bit doubles instead, which is fast at some cost in
/// accuracy (FEX warns of possible rendering bugs). Our FEX still rounds to
/// single precision when the x87 precision control asks for 24 bits, as D3D9
/// sets it when it creates a device. FEX takes the option from
/// `FEX_X87REDUCEDPRECISION`, parsed as an integer (so "1", not "true"), and
/// reads it from the Windows environment block when its process init runs,
/// after this library has written its env there.
///
/// It matches every process, native ARM64 ones included. FEX is not loaded in
/// those, so the variable does nothing there, but a child they start inherits
/// it, and that child may be x86. A rule for one game with
/// `env=FEX_X87REDUCEDPRECISION=` removes it again, and the variable exported
/// where the process starts wins over it.
#[must_use]
pub fn fex_reduced_x87_precision() -> Rule {
    Rule {
        exe: Exe::Any,
        env: vec![EnvEntry::literal("FEX_X87REDUCEDPRECISION", "1")],
        ..Rule::default()
    }
}

/// Mesa's EGL for OpenGL in every process.
///
/// OpenGL goes through Mesa's EGL with Zink on KosmicKrisp, which reaches
/// OpenGL 4.6 and the compatibility profile, instead of Apple's OpenGL,
/// upstream Wine's, which stops at 4.1 core and 2.1 compatibility.
/// winemac.drv reads the `opengl` setting through ntdll when a process first
/// uses OpenGL, long after compatdb has handed it over; unset, it uses
/// Apple's OpenGL. With egl it does not fall back: where Zink has no GPU, as
/// on a Mac without a Metal 4 GPU, the process gets no OpenGL, and
/// `opengl = cgl` gives it Apple's.
///
/// It matches every process, like the other [`defaults`]. A rule for one
/// program with `opengl = cgl` gives that program Apple's OpenGL, and only
/// that program: the value is the process's own, so the programs it starts
/// keep this default. A `*` rule with it gives every process Apple's.
#[must_use]
pub fn opengl_egl() -> Rule {
    Rule {
        exe: Exe::Any,
        opengl: Setting::Set(OpenGl::Egl),
        ..Rule::default()
    }
}

/// No-execute stays on in every 32-bit process.
///
/// i386 code runs under FEX. Wine turns no-execute off for a 32-bit process
/// when its executable or a DLL it loads lacks the `NX_COMPAT` flag, as many
/// older games do, and FEX then has to retranslate code whenever writable
/// memory changes. With `force_nx`, our ntdll keeps no-execute permanently on
/// instead; it reads the value the first time the process asks for or
/// changes its execute flags, long after compatdb has handed it over. A
/// 64-bit process always has no-execute on, so the setting does nothing
/// there.
///
/// A program that runs code from memory it did not make executable crashes
/// with it. That includes programs built with the ATL of Visual Studio 2003,
/// such as game configurators and launchers, whose window thunks live on the
/// heap: Windows emulates such thunks, but in a WoW64 process under FEX
/// neither Wine nor FEX does, so the first window message faults.
///
/// It matches every process, like the other [`defaults`]. A rule for one
/// program with `force_nx = false` gives that program Wine's behaviour
/// again, and only that program: the value is the process's own, so the
/// programs it starts keep this default. A `*` rule with it does that for
/// every process.
#[must_use]
pub fn force_nx() -> Rule {
    Rule {
        exe: Exe::Any,
        force_nx: Setting::Set(true),
        ..Rule::default()
    }
}

/// Keeps Wine from prompting to install Mono and Gecko, in wineboot and in
/// any process that loads either.
///
/// The override disables both modules, not only the prompts. ntdll consults
/// the overrides compatdb adds before the registry `DllOverrides` keys, so an
/// installed wine-mono, Gecko or native .NET selected there stays disabled
/// too. A prefix that needs one adds a later entry for the module, such as
/// `mscoree=n,b`, or hands it back to the registry with `mscoree=default`, in
/// a `*` rule or a rule for the program.
#[must_use]
pub fn no_mono_or_gecko() -> Rule {
    Rule {
        exe: Exe::Any,
        dll_overrides: vec![DllOverride::literal("mscoree,mshtml=")],
        ..Rule::default()
    }
}

/// wined3d for the DXGI family in the Rockstar Games Launcher.
///
/// The Rockstar Games Launcher needs a real D3D10.1 device, which DXMT, the
/// default, does not provide; wined3d does. Matched by its version resource,
/// not its path, so it works wherever it is installed and never hits another
/// game's Launcher.exe.
#[must_use]
pub fn rockstar_games_launcher() -> Rule {
    Rule {
        exe: Exe::literal("Launcher.exe"),
        company: Some(Fingerprint::literal("Rockstar Games")),
        product: Some(Fingerprint::literal("Rockstar Games Launcher")),
        dxgi: Setting::Set(Dxgi::Wined3d),
        ..Rule::default()
    }
}

/// An in-process GPU, DXMT and no Metal HUD for the Rockstar Games
/// Launcher's Social Club helper.
///
/// The launcher's embedded browser (CEF): its GPU process cannot paint into a
/// window owned by another process under winemac, so the sign-in page stays
/// blank. Keeping the drawing in-process fixes it; the child --type=
/// processes inherit the switches. It is pinned to DXMT to match Steam's CEF
/// helper ([`steam_web_helper`]), which embeds the browser the same way. The
/// Metal HUD is for games and is noise on the launcher UI; it is turned off
/// here and not on Launcher.exe because games started from the launcher
/// would inherit the variable.
#[must_use]
pub fn social_club_helper() -> Rule {
    Rule {
        exe: Exe::literal("SocialClubHelper.exe"),
        company: Some(Fingerprint::literal("Take-Two Interactive Software")),
        product: Some(Fingerprint::literal("Social Club UI")),
        dxgi: Setting::Set(Dxgi::Dxmt),
        arguments: vec![Argument::literal("--in-process-gpu")],
        env: vec![EnvEntry::literal("MTL_HUD_ENABLED", "0")],
        ..Rule::default()
    }
}

/// An in-process GPU, DXMT and no Metal HUD for Steam's web helper.
///
/// Steam's embedded browser (CEF): its GPU process cannot paint into the
/// browser's window (blank UI), so keep the GPU in-process. GPU rendering
/// stays on and goes through DXMT, which the Steam client runs well on; the
/// dxgi pin keeps it there over the `*` rules, built-in or from
/// `WINE_COMPATDB`, while a `WINE_COMPATDB` rule for the helper that
/// selects another tree moves it. The Metal HUD is off for the same reason
/// as on the Social Club helper ([`social_club_helper`]); steam.exe, which
/// starts the games, has no rule.
#[must_use]
pub fn steam_web_helper() -> Rule {
    Rule {
        exe: Exe::literal("steamwebhelper.exe"),
        company: Some(Fingerprint::literal("Valve Corporation")),
        product: Some(Fingerprint::literal("Steam Client WebHelper")),
        dxgi: Setting::Set(Dxgi::Dxmt),
        arguments: vec![Argument::literal("--in-process-gpu")],
        env: vec![EnvEntry::literal("MTL_HUD_ENABLED", "0")],
        ..Rule::default()
    }
}

/// The video memory GTA IV assumes.
///
/// GTA IV (Complete Edition). The switch overrides the game's own
/// video-memory detection, which mis-reads the reported figure and falls back
/// to 512 MB; 2048 unlocks every quality tier while staying inside the 32-bit
/// address space. Nothing else is needed here: the D3D9 quirks this game
/// wants are backend behaviour, so they live in mtld3d's own built-in profile
/// for GTAIV.exe and apply whichever launcher started it.
#[must_use]
pub fn gta_iv() -> Rule {
    Rule {
        exe: Exe::literal("GTAIV.exe"),
        company: Some(Fingerprint::literal("Rockstar Games")),
        product: Some(Fingerprint::literal("Grand Theft Auto IV")),
        arguments: vec![Argument::literal("-availablevidmem 2048.0")],
        ..Rule::default()
    }
}

/// The NosTale client is DPI-unaware.
///
/// NosTale rasterises its text with GDI into fixed-grid glyph atlases and
/// fixed-size UI boxes laid out for 96 DPI. Marked DPI-aware at 192 DPI
/// (retina), GDI hands it fonts twice as large, so wide glyphs overrun their
/// atlas slots (clipped "m" and "w") and text overflows its boxes. Left
/// DPI-unaware, Wine gives it 96 DPI fonts and scales its window, as Windows
/// does at 200%. The client's version resource names only the company, so
/// that is the whole fingerprint.
#[must_use]
pub fn nostale() -> Rule {
    Rule {
        exe: Exe::literal("NostaleClientX.exe"),
        company: Some(Fingerprint::literal("Entwell")),
        dpi_aware: Setting::Set(false),
        ..Rule::default()
    }
}

/// Age of Empires II HD is DPI-unaware.
///
/// Age of Empires II HD draws its UI at a fixed size in pixels. Marked
/// DPI-aware at 192 DPI (retina), it runs at the display's full resolution
/// and its UI comes out too small to use; left DPI-unaware, Wine scales its
/// window, as Windows does at 200%, and the UI has its intended size. Its
/// manifest does not declare DPI awareness, but the game looks up
/// `SetProcessDPIAware` and likely calls it at run time. The rule still wins:
/// win32u sets the process awareness from it when it initializes, and
/// refuses every later change. The fingerprint is the company alone: the
/// product name of the build this was checked with, `age2_x1`, is the
/// original game's, and newer Steam builds may have changed it.
#[must_use]
pub fn age_of_empires_ii_hd() -> Rule {
    Rule {
        exe: Exe::literal("AoK HD.exe"),
        company: Some(Fingerprint::literal("Microsoft Corporation")),
        dpi_aware: Setting::Set(false),
        ..Rule::default()
    }
}

/// The 4 GB address space for 32-bit World of Warcraft clients.
///
/// Every 32-bit WoW client with these version strings gets the 4 GB address
/// space. Under Wine the client shares its 2 GB with Wine itself and the
/// Direct3D layers, so it runs out long before it would on Windows. The
/// clients checked are 1.12.1 and 3.3.5a: both carry these three strings, and
/// their unpatched builds lack the large-address-aware flag. The matcher
/// cannot pin `FileVersion`, so other 32-bit clients with the same strings get
/// it too. Patched private-server clients, such as Turtle WoW's, already
/// have the flag and need no rule; a client that has it loses nothing. A
/// 64-bit client is not affected.
#[must_use]
pub fn world_of_warcraft() -> Rule {
    Rule {
        exe: Exe::literal("WoW.exe"),
        company: Some(Fingerprint::literal("Blizzard Entertainment")),
        product: Some(Fingerprint::literal("World of Warcraft")),
        original_filename: Some(Fingerprint::literal("WoW.exe")),
        large_address_aware: Setting::Set(true),
        ..Rule::default()
    }
}

/// A name a built-in rule had while each had one.
///
/// A launcher's settings written for those rules may still use one to
/// extend a rule or turn it off. The rules have no names any more, so such
/// a name reaches nothing; [`former`] gives what does the same now.
///
/// Its `Display` form says that, for example `"dxmt" was the name of a
/// built-in rule of compatdb.so, and the built-in rules have no names any
/// more, so a rule can no longer extend or turn off one by naming it`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Former {
    /// The name, in lowercase.
    name: &'static str,
    /// What the rule of that name did, and so what does it now.
    did: Did,
}

/// What a former built-in rule did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Did {
    /// Set the scalar setting of this key in every process.
    Setting(Key),
    /// Anything else: what to do instead of extending it, if that differs
    /// from what to do instead of turning it off, and that.
    Other {
        /// The clause that says what to do instead of extending the rule.
        extend: Option<&'static str>,
        /// The clause that says what to do instead of turning it off.
        off: &'static str,
    },
}

/// What changes what a former built-in rule for one program set.
const PROGRAM_RULE: Did = Did::Other {
    extend: None,
    off: "to change what a built-in rule sets for one program, give that program's exe a rule \
          that sets the same keys, or default for upstream Wine's behaviour",
};

/// The former names of the built-in rules.
const FORMER: &[Former] = &[
    Former {
        name: "dxmt",
        did: Did::Setting(Key::Dxgi),
    },
    Former {
        name: "mtld3d",
        did: Did::Setting(Key::D3d9),
    },
    Former {
        name: "dpi-aware",
        did: Did::Setting(Key::DpiAware),
    },
    Former {
        name: "force-nx",
        did: Did::Setting(Key::ForceNx),
    },
    Former {
        name: "gcmouse",
        did: Did::Setting(Key::Gcmouse),
    },
    Former {
        name: "opengl-egl",
        did: Did::Setting(Key::OpenGl),
    },
    Former {
        name: "no-mono-gecko",
        did: Did::Other {
            extend: Some(
                "set dll_overrides instead, in a rule matching every process or in a rule for \
                 one program",
            ),
            off: "dll_overrides set to \"mscoree,mshtml=default\" in a rule matching every \
                  process takes the built-in overrides of Mono and Gecko away instead",
        },
    },
    Former {
        name: "fex-x87-reduced-precision",
        did: Did::Other {
            extend: Some(
                "set FEX_X87REDUCEDPRECISION in the env of a rule instead, one matching every \
                 process or one for one program",
            ),
            off: "FEX_X87REDUCEDPRECISION=0 in the environment Wine is started with gives every \
                  process FEX's full x87 precision instead, as does a rule matching every \
                  process whose env removes FEX_X87REDUCEDPRECISION",
        },
    },
    Former {
        name: "vkd3d-proton-min-latency",
        did: Did::Other {
            extend: None,
            off: "compatdb.so no longer has that rule, so there is nothing left to change or turn \
                  off; remove it",
        },
    },
    Former {
        name: "large-address-aware",
        did: Did::Other {
            extend: None,
            off: "compatdb.so no longer gives every 32-bit process 4 GB: the executable's \
                  large-address-aware flag decides, and large_address_aware set to true gives 4 \
                  GB, in a rule matching every process to every 32-bit process or in a rule for \
                  one program to that program",
        },
    },
    Former {
        name: "rockstar-launcher",
        did: PROGRAM_RULE,
    },
    Former {
        name: "rockstar-social-club-ui",
        did: PROGRAM_RULE,
    },
    Former {
        name: "steam-web-helper",
        did: PROGRAM_RULE,
    },
    Former {
        name: "gta-iv",
        did: PROGRAM_RULE,
    },
    Former {
        name: "nostale",
        did: PROGRAM_RULE,
    },
    Former {
        name: "aoe2-hd",
        did: PROGRAM_RULE,
    },
];

/// The former built-in rule called `name`, compared
/// ASCII-case-insensitively as `compatdb.so` compared the names, or `None`
/// when no built-in rule had that name.
#[must_use]
pub fn former(name: &str) -> Option<Former> {
    FORMER
        .iter()
        .find(|former| former.name.eq_ignore_ascii_case(name))
        .copied()
}

impl Former {
    /// The name, in lowercase.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// What to do instead of extending the rule by its name, a clause a
    /// launcher can print after its own words, for example `set dxgi
    /// instead, in a rule matching every process or in a rule for one
    /// program`.
    #[must_use]
    pub fn instead_of_extending(self) -> String {
        match self.did {
            Did::Setting(key) => format!(
                "set {key} instead, in a rule matching every process or in a rule for one program"
            ),
            Did::Other {
                extend: Some(extend),
                ..
            } => extend.to_owned(),
            Did::Other { extend: None, off } => off.to_owned(),
        }
    }

    /// What to do instead of turning the rule off by its name, a clause
    /// like [`Former::instead_of_extending`].
    #[must_use]
    pub fn instead_of_turning_off(self) -> String {
        match self.did {
            Did::Setting(key) => format!(
                "{key} set to default in a rule matching every process hands the setting back to \
                 upstream Wine in every process instead, except where a built-in rule for one \
                 program sets it"
            ),
            Did::Other { off, .. } => off.to_owned(),
        }
    }
}

impl fmt::Display for Former {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} was the name of a built-in rule of compatdb.so, and the built-in rules have no \
             names any more, so a rule can no longer extend or turn off one by naming it",
            self.name
        )
    }
}

#[cfg(test)]
mod tests;
