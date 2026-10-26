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
//! `WINE_COMPATDB` rule that sets the same key, overrides.

use compatdb_core::{ANY_EXE, D3d9, Dxgi, OpenGl, Rule, Table};

/// The rules the bundle ships with: the [`defaults`] for every process, then
/// the rules for one program each.
pub fn table() -> Table {
    Table {
        rules: defaults().into_iter().chain(programs()).collect(),
    }
}

/// The rules that match every process, each turning on one behaviour that
/// differs from upstream Wine.
fn defaults() -> Vec<Rule> {
    vec![
        // D3D10, D3D11 and D3D12 go through DXMT, which translates them to
        // Metal directly, instead of through wined3d, upstream Wine's
        // implementation, which draws through OpenGL or Vulkan and reaches a
        // lower feature level and frame rate on a Mac. D3D12 is vkd3d-proton
        // in the same tree, which presents through DXMT's dxgi.dll; the
        // wined3d tree has no D3D12 at all. A rule for one program with
        // `dxgi = wined3d` gives it wined3d, as the Rockstar launcher's does,
        // and a `*` rule with it gives every process wined3d.
        Rule {
            exe: ANY_EXE.into(),
            dxgi: Some(Dxgi::Dxmt),
            ..Rule::default()
        },
        // D3D9 goes through mtld3d, which translates it to Metal directly,
        // instead of through wined3d, for the same reason. A rule for one
        // program with `d3d9 = wined3d` gives it wined3d, and a `*` rule with
        // it gives every process wined3d.
        Rule {
            exe: ANY_EXE.into(),
            d3d9: Some(D3d9::Mtld3d),
            ..Rule::default()
        },
        // Programs under this Wine are treated as DPI-aware unless a rule
        // says otherwise. An unaware program on a scaled desktop gets its
        // window and mouse coordinates scaled while display modes are
        // not, so a game that sizes itself from the mode list draws and
        // reads the pointer in two different coordinate spaces.
        Rule {
            exe: ANY_EXE.into(),
            dpi_aware: Some(true),
            ..Rule::default()
        },
        // The Mac driver takes the mouse movement of raw input (WM_INPUT)
        // and DirectInput from GameController's GCMouse, which reports the
        // device's own counts. Otherwise that movement is the cursor's,
        // which macOS has already accelerated and, on a retina display,
        // scaled, so mouselook in a game that reads raw input or DirectInput
        // turns further the faster the hand moves. winemac.drv reads the
        // value through ntdll when it reads its settings, and without one
        // keeps upstream Wine's behaviour, the cursor's movement.
        //
        // It matches every process, like the rules above. A rule for one
        // game with `gcmouse = false` gives it the cursor's movement again:
        // for a game whose sensitivity was tuned to that, a mouse that
        // GameController reports twice, or a regression. A `*` rule with it
        // does the same for every process.
        Rule {
            exe: ANY_EXE.into(),
            gcmouse: Some(true),
            ..Rule::default()
        },
        // x86 code runs under FEX, which by default emulates x87
        // floating-point code, which games built for 32-bit x86 use heavily,
        // at its full 80-bit precision, and such games run slowly that way.
        // Its X87ReducedPrecision option computes with 64-bit doubles
        // instead, which is fast at some cost in accuracy (FEX warns of
        // possible rendering bugs). Our FEX still rounds to single precision
        // when the x87 precision control asks for 24 bits, as D3D9 sets it
        // when it creates a device. FEX takes the option from
        // FEX_X87REDUCEDPRECISION, parsed as an integer (so "1", not
        // "true"), and reads it from the Windows environment block when its
        // process init runs, after this library has written its env there.
        //
        // It matches every process, native ARM64 ones included. FEX is not
        // loaded in those, so the variable does nothing there, but a child
        // they start inherits it, and that child may be x86. A rule for one
        // game with `env=FEX_X87REDUCEDPRECISION=` removes it again, and the
        // variable exported where the process starts wins over it.
        Rule {
            exe: ANY_EXE.into(),
            env: vec![("FEX_X87REDUCEDPRECISION".into(), "1".into())],
            ..Rule::default()
        },
        // OpenGL goes through Mesa's EGL with Zink on KosmicKrisp, which
        // reaches OpenGL 4.6 and the compatibility profile, instead of
        // Apple's OpenGL, upstream Wine's, which stops at 4.1 core and 2.1
        // compatibility. winemac.drv reads the `opengl` setting through ntdll
        // when a process first uses OpenGL, long after compatdb has handed it
        // over; unset, it uses Apple's OpenGL. With egl it does not fall
        // back: where Zink has no GPU, as on a Mac without a Metal 4 GPU, the
        // process gets no OpenGL, and `opengl = cgl` gives it Apple's.
        //
        // It matches every process, like the rules above. A rule for one
        // program with `opengl = cgl` gives that program Apple's OpenGL, and
        // only that program: the value is the process's own, so the programs
        // it starts keep this default. A `*` rule with it gives every
        // process Apple's.
        Rule {
            exe: ANY_EXE.into(),
            opengl: Some(OpenGl::Egl),
            ..Rule::default()
        },
        // i386 code runs under FEX. Wine turns no-execute off for a 32-bit
        // process when its executable or a DLL it loads lacks the NX_COMPAT
        // flag, as many older games do, and FEX then has to retranslate code
        // whenever writable memory changes. With `force_nx`, our ntdll keeps
        // no-execute permanently on instead; it reads the value the first
        // time the process asks for or changes its execute flags, long after
        // compatdb has handed it over. A 64-bit process always has
        // no-execute on, so the setting does nothing there.
        //
        // A program that runs code from memory it did not make executable
        // crashes with it. That includes programs built with the ATL of
        // Visual Studio 2003, such as game configurators and launchers,
        // whose window thunks live on the heap: Windows emulates such
        // thunks, but in a WoW64 process under FEX neither Wine nor FEX
        // does, so the first window message faults.
        //
        // It matches every process, like the rules above. A rule for one
        // program with `force_nx = false` gives that program Wine's
        // behaviour again, and only that program: the value is the
        // process's own, so the programs it starts keep this default. A `*`
        // rule with it does that for every process.
        Rule {
            exe: ANY_EXE.into(),
            force_nx: Some(true),
            ..Rule::default()
        },
        // Keeps Wine from prompting to install Mono and Gecko, in
        // wineboot and in any process that loads either.
        //
        // The override disables both modules, not only the prompts. ntdll
        // consults the overrides compatdb adds before the registry
        // DllOverrides keys, so an installed wine-mono, Gecko or native
        // .NET selected there stays disabled too. A prefix that needs one
        // adds a later entry for the module, such as `mscoree=n,b`, or hands
        // it back to the registry with `mscoree=default`, in a `*` rule or a
        // rule for the program.
        Rule {
            exe: ANY_EXE.into(),
            dll_overrides: vec!["mscoree,mshtml=".into()],
            ..Rule::default()
        },
    ]
}

/// The rules for one program each, which beat the [`defaults`] for it.
fn programs() -> Vec<Rule> {
    vec![
        // The Rockstar Games Launcher needs a real D3D10.1 device, which
        // DXMT, the default, does not provide; wined3d does. Matched by its
        // version resource, not its path, so it works wherever it is
        // installed and never hits another game's Launcher.exe.
        Rule {
            exe: "Launcher.exe".into(),
            company: Some("Rockstar Games".into()),
            product: Some("Rockstar Games Launcher".into()),
            dxgi: Some(Dxgi::Wined3d),
            ..Rule::default()
        },
        // The launcher's embedded browser (CEF): its GPU process cannot
        // paint into a window owned by another process under winemac, so
        // the sign-in page stays blank. Keeping the drawing in-process
        // fixes it; the child --type= processes inherit the switches.
        // It is pinned to DXMT to match Steam's CEF helper below, which
        // embeds the browser the same way. The Metal HUD is for games and
        // is noise on the launcher UI; it is turned off here and not on
        // Launcher.exe because games started from the launcher would
        // inherit the variable.
        Rule {
            exe: "SocialClubHelper.exe".into(),
            company: Some("Take-Two Interactive Software".into()),
            product: Some("Social Club UI".into()),
            dxgi: Some(Dxgi::Dxmt),
            arguments: vec!["--in-process-gpu".into()],
            env: vec![("MTL_HUD_ENABLED".into(), "0".into())],
            ..Rule::default()
        },
        // Steam's embedded browser (CEF): its GPU process cannot paint into
        // the browser's window (blank UI), so keep the GPU in-process. GPU
        // rendering stays on and goes through DXMT, which the Steam client
        // runs well on; the dxgi pin keeps it there over the built-in `*`
        // rule, while a WINE_COMPATDB rule that selects another tree, even a
        // `*` one, moves it too. The Metal HUD is off for the same
        // reason as on the Social Club helper; steam.exe, which starts the
        // games, has no rule.
        Rule {
            exe: "steamwebhelper.exe".into(),
            company: Some("Valve Corporation".into()),
            product: Some("Steam Client WebHelper".into()),
            dxgi: Some(Dxgi::Dxmt),
            arguments: vec!["--in-process-gpu".into()],
            env: vec![("MTL_HUD_ENABLED".into(), "0".into())],
            ..Rule::default()
        },
        // GTA IV (Complete Edition). The switch overrides the game's own
        // video-memory detection, which mis-reads the reported figure and
        // falls back to 512 MB; 2048 unlocks every quality tier while
        // staying inside the 32-bit address space. Nothing else is needed
        // here: the D3D9 quirks this game wants are backend behaviour, so
        // they live in mtld3d's own built-in profile for GTAIV.exe and
        // apply whichever launcher started it.
        Rule {
            exe: "GTAIV.exe".into(),
            company: Some("Rockstar Games".into()),
            product: Some("Grand Theft Auto IV".into()),
            arguments: vec!["-availablevidmem 2048.0".into()],
            ..Rule::default()
        },
        // NosTale rasterises its text with GDI into fixed-grid glyph
        // atlases and fixed-size UI boxes laid out for 96 DPI. Marked
        // DPI-aware at 192 DPI (retina), GDI hands it fonts twice as
        // large, so wide glyphs overrun their atlas slots (clipped "m"
        // and "w") and text overflows its boxes. Left DPI-unaware, Wine
        // gives it 96 DPI fonts and scales its window, as Windows does at
        // 200%. The client's version resource names only the company, so
        // that is the whole fingerprint.
        Rule {
            exe: "NostaleClientX.exe".into(),
            company: Some("Entwell".into()),
            dpi_aware: Some(false),
            ..Rule::default()
        },
        // Age of Empires II HD draws its UI at a fixed size in pixels.
        // Marked DPI-aware at 192 DPI (retina), it runs at the display's
        // full resolution and its UI comes out too small to use; left
        // DPI-unaware, Wine scales its window, as Windows does at 200%, and
        // the UI has its intended size. Its manifest does not declare DPI
        // awareness, but the game looks up SetProcessDPIAware and likely
        // calls it at run time. The rule still wins: win32u sets the
        // process awareness from it when it initializes, and refuses every
        // later change. The fingerprint is the company alone: the product
        // name of the build this was checked with, age2_x1, is the original
        // game's, and newer Steam builds may have changed it.
        Rule {
            exe: "AoK HD.exe".into(),
            company: Some("Microsoft Corporation".into()),
            dpi_aware: Some(false),
            ..Rule::default()
        },
        // Every 32-bit WoW client with these version strings gets the 4 GB
        // address space. Under Wine the client shares its 2 GB with Wine
        // itself and the Direct3D layers, so it runs out long before it
        // would on Windows. The clients checked are 1.12.1 and 3.3.5a: both
        // carry these three strings, and their unpatched builds lack the
        // large-address-aware flag. The matcher cannot pin FileVersion, so
        // other 32-bit clients with the same strings get it too. Patched
        // private-server clients, such as Turtle WoW's, already have the
        // flag and need no rule; a client that has it loses nothing. A
        // 64-bit client is not affected.
        Rule {
            exe: "WoW.exe".into(),
            company: Some("Blizzard Entertainment".into()),
            product: Some("World of Warcraft".into()),
            original_filename: Some("WoW.exe".into()),
            large_address_aware: Some(true),
            ..Rule::default()
        },
    ]
}

#[cfg(test)]
mod tests;
