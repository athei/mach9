//! The built-in rules, compiled into `compatdb.so`.
//!
//! This is static data the library owns, so it never travels through the
//! environment. Only overrides are serialized into `WINE_COMPATDB`; the
//! library overlays them onto these rules by name.
//!
//! Every rule that names an executable also carries a version fingerprint, so
//! none of them can catch an unrelated program that happens to share a file
//! name. The exceptions are `dxmt`, `mtld3d`, `dpi-aware`, `gcmouse`,
//! `fex-x87-reduced-precision`, `opengl-egl`, `force-nx` and
//! `no-mono-gecko`, which match every process on purpose: each is a default
//! that a more specific rule can override and that can be dropped by name.

use compatdb_table::{ANY_EXE, D3d9, Dxgi, OpenGl, Rule, Table};

/// The rules the bundle ships with: the [`defaults`] for every process, then
/// the rules for one program each.
pub fn table() -> Table {
    Table {
        rules: defaults()
            .into_iter()
            .chain(programs())
            .map(with_env_defaults)
            .collect(),
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
        // and dropping this rule gives every process wined3d.
        Rule {
            name: "dxmt".into(),
            exe: ANY_EXE.into(),
            dxgi: Some(Dxgi::Dxmt),
            ..Rule::default()
        },
        // D3D9 goes through mtld3d, which translates it to Metal directly,
        // instead of through wined3d, for the same reason. A rule for one
        // program with `d3d9 = wined3d` gives it wined3d, and dropping this
        // rule gives every process wined3d.
        Rule {
            name: "mtld3d".into(),
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
            name: "dpi-aware".into(),
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
        // GameController reports twice, or a regression. Dropping the rule
        // by name does the same for every process.
        Rule {
            name: "gcmouse".into(),
            exe: ANY_EXE.into(),
            gcmouse: Some(true),
            ..Rule::default()
        },
        // x86 code runs under FEX, which by default emulates x87
        // floating-point code, which games built for 32-bit x86 use heavily,
        // at its full 80-bit precision, and such games run slowly that way.
        // Its X87ReducedPrecision option computes with 64-bit precision
        // instead, which is fast at some cost in accuracy (FEX warns of
        // possible rendering bugs). FEX takes the option from
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
            name: "fex-x87-reduced-precision".into(),
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
        // it starts keep this default. Dropping the rule gives every process
        // Apple's.
        Rule {
            name: "opengl-egl".into(),
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
        // process's own, so the programs it starts keep this default.
        // Dropping the rule does that for every process.
        Rule {
            name: "force-nx".into(),
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
        // either drops this rule (`name=no-mono-gecko;enabled=false`) or
        // adds a later entry for the module, such as `mscoree=n,b`.
        Rule {
            name: "no-mono-gecko".into(),
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
            name: "rockstar-launcher".into(),
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
            name: "rockstar-social-club-ui".into(),
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
        // runs well on; the dxgi pin keeps it there even when a wildcard
        // rule selects another tree. The Metal HUD is off for the same
        // reason as on the Social Club helper; steam.exe, which starts the
        // games, has no rule.
        Rule {
            name: "steam-web-helper".into(),
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
            name: "gta-iv".into(),
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
            name: "nostale".into(),
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
            name: "aoe2-hd".into(),
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
            name: "world-of-warcraft".into(),
            exe: "WoW.exe".into(),
            company: Some("Blizzard Entertainment".into()),
            product: Some("World of Warcraft".into()),
            original_filename: Some("WoW.exe".into()),
            large_address_aware: Some(true),
            ..Rule::default()
        },
    ]
}

/// `rule` with its env entries marked as defaults when it matches every
/// process. A variable the process already has wins over such an entry:
/// whoever starts the process can export the variable to choose for itself.
/// A rule for one program, or any rule from `WINE_COMPATDB`, replaces the
/// value.
fn with_env_defaults(rule: Rule) -> Rule {
    let env_defaults = if rule.specificity() == 0 {
        rule.env.len()
    } else {
        0
    };
    Rule {
        env_defaults,
        ..rule
    }
}

#[cfg(test)]
mod tests {
    use compatdb_table::{Origin, Resolution, VersionInfo, Wined3d};

    use super::*;

    /// What a test returns, so a missing value fails it through `?` rather
    /// than a panic in the test body.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Record in `resolution` that `rule` set `key`, as resolving a more
    /// specific rule does.
    fn set_by(resolution: &mut Resolution, key: &'static str, rule: &str) {
        resolution.set_by.retain(|(k, _)| *k != key);
        resolution.set_by.push((
            key,
            Origin {
                rule: rule.to_string(),
                changed: false,
            },
        ));
    }

    /// The env entries a process gets applied when it already has the
    /// variables in `has`, as `NAME=value`, and the names of those left
    /// alone.
    fn applied(resolution: &Resolution, has: &[&str]) -> (Vec<String>, Vec<String>) {
        let steps = resolution.env_steps(|name| has.contains(&name));
        let (set, kept): (Vec<_>, Vec<_>) = steps.into_iter().partition(|s| s.applied);
        (
            set.into_iter()
                .map(|s| format!("{}={}", s.name, s.value))
                .collect(),
            kept.into_iter().map(|s| s.name).collect(),
        )
    }

    #[test]
    fn a_variable_the_caller_set_wins_over_a_builtin_default() {
        let table = table();
        let game = table.resolve("game.exe", &VersionInfo::default());
        // Unset, the defaults apply.
        let (set, kept) = applied(&game, &[]);
        assert_eq!(set, ["FEX_X87REDUCEDPRECISION=1"]);
        assert_eq!(kept, [] as [String; 0]);
        // Exported by the caller, the default leaves it alone.
        let (set, kept) = applied(&game, &["FEX_X87REDUCEDPRECISION"]);
        assert_eq!(set, [] as [String; 0]);
        assert_eq!(kept, ["FEX_X87REDUCEDPRECISION"]);
        // The steps name the rule of each entry.
        let steps = game.env_steps(|_| false);
        assert_eq!(
            steps.first().map(|s| s.origin.rule.as_str()),
            Some("fex-x87-reduced-precision")
        );
    }

    #[test]
    fn a_rule_for_one_game_or_from_the_launcher_replaces_what_the_caller_set() {
        // A rule for one game, a launcher's own `*` rule and an override of
        // the built-in rule by name all replace the caller's value, since
        // whoever wrote them chose it for that process.
        for text in [
            "name=my-game;exe=game.exe;env=FEX_X87REDUCEDPRECISION=0",
            "name=global;exe=*;env=FEX_X87REDUCEDPRECISION=0",
            "name=fex-x87-reduced-precision;env=FEX_X87REDUCEDPRECISION=0",
        ] {
            let table = with_overrides(text);
            let game = table.resolve("game.exe", &VersionInfo::default());
            let (set, kept) = applied(&game, &["FEX_X87REDUCEDPRECISION"]);
            assert_eq!(kept, ["FEX_X87REDUCEDPRECISION"], "{text}");
            assert!(
                set.contains(&"FEX_X87REDUCEDPRECISION=0".to_string()),
                "{text}: {set:?}"
            );
        }
        // Only the `*` rules' own entries are defaults.
        assert!(
            table()
                .rules
                .iter()
                .all(|r| r.env_defaults == if r.exe == ANY_EXE { r.env.len() } else { 0 })
        );
    }

    /// The rule of `table` named `name`.
    fn rule<'a>(table: &'a Table, name: &str) -> Result<&'a Rule, String> {
        table
            .rules
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| format!("no rule {name}"))
    }

    /// The name of the rule that asks FEX for reduced x87 precision.
    const FEX_RULE: &str = "fex-x87-reduced-precision";

    /// The name of the rule that asks the Mac driver for Mesa's EGL.
    const GL_RULE: &str = "opengl-egl";

    /// The name of the rule that keeps no-execute permanently on.
    const NX_RULE: &str = "force-nx";

    /// The names of the rules that match every process, in table order.
    fn wildcards() -> Vec<String> {
        [
            "dxmt",
            "mtld3d",
            "dpi-aware",
            "gcmouse",
            FEX_RULE,
            GL_RULE,
            NX_RULE,
            "no-mono-gecko",
        ]
        .map(String::from)
        .to_vec()
    }

    /// The env of the FEX rule. The variable is spelled out here to guard
    /// against a typo: FEX matches the name exactly and parses the value as
    /// an integer.
    fn fex_env() -> Vec<(String, String)> {
        vec![("FEX_X87REDUCEDPRECISION".to_string(), "1".to_string())]
    }

    /// The wildcard rules that set env, each with its entries, in table
    /// order.
    fn wildcard_env_rules() -> Vec<(&'static str, Vec<(String, String)>)> {
        vec![(FEX_RULE, fex_env())]
    }

    /// The env every process gets from the built-in rules without the rule
    /// of the given name.
    fn wildcard_env_without(name: &str) -> Vec<(String, String)> {
        wildcard_env_rules()
            .into_iter()
            .filter(|(n, _)| *n != name)
            .flat_map(|(_, env)| env)
            .collect()
    }

    /// The env every process gets from the built-in rules.
    fn wildcard_env() -> Vec<(String, String)> {
        wildcard_env_rules()
            .into_iter()
            .flat_map(|(_, env)| env)
            .collect()
    }

    /// The DLL overrides every process gets from the built-in rules: Mono
    /// and Gecko are disabled.
    fn wildcard_overrides() -> Vec<String> {
        vec!["mscoree,mshtml=".to_string()]
    }

    /// `wildcard_env()` followed by `more`.
    fn wildcard_env_and(more: &[(String, String)]) -> Vec<(String, String)> {
        let mut env = wildcard_env();
        env.extend_from_slice(more);
        env
    }

    /// `wildcards()` without the rule of the given name.
    fn wildcards_without(name: &str) -> Vec<String> {
        wildcards().into_iter().filter(|n| n != name).collect()
    }

    /// `wildcards()` followed by one more specific rule.
    fn wildcards_and(name: &str) -> Vec<String> {
        let mut names = wildcards();
        names.push(name.to_string());
        names
    }

    /// The built-in table with the rules in `text` laid on top, which must
    /// parse cleanly and all find a home.
    fn with_overrides(text: &str) -> Table {
        let mut table = table();
        let (over, diagnostics) = Table::parse(text);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(table.overlay(over), [] as [String; 0]);
        table
    }

    #[test]
    fn the_table_asks_fex_for_reduced_x87_precision() -> TestResult {
        let table = table();
        let fex = rule(&table, FEX_RULE)?;
        assert_eq!(fex.exe, ANY_EXE);
        assert_eq!(fex.env, fex_env());
        // Every process gets it: x86_64, i386 and native ARM64 alike, since
        // rules do not match on the arch.
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.env, wildcard_env());
        Ok(())
    }

    #[test]
    fn a_game_rule_or_disabling_by_name_turns_reduced_x87_precision_off() -> TestResult {
        // For one game: an empty value, which removes the variable, folds
        // after the default.
        let table = with_overrides("name=old;exe=old.exe;env=FEX_X87REDUCEDPRECISION=");
        let old = table.resolve("Old.exe", &VersionInfo::default());
        let removed = ("FEX_X87REDUCEDPRECISION".to_string(), String::new());
        assert_eq!(old.env, wildcard_env_and(&[removed]));
        let block: Vec<u16> = "FEX_X87REDUCEDPRECISION=1\0OTHER=x\0\0"
            .encode_utf16()
            .collect();
        let merged = compatdb_table::merge_env(&block, &old.env).ok_or("unchanged")?;
        assert_eq!(String::from_utf16_lossy(&merged), "OTHER=x\0\0");
        // Another process keeps it.
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.env, wildcard_env());
        // For every process: drop the rule. The other entries stay.
        let table = with_overrides(&format!("name={FEX_RULE};enabled=false"));
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.env, wildcard_env_without(FEX_RULE));
        assert_eq!(game.matched, wildcards_without(FEX_RULE));
        Ok(())
    }

    #[test]
    fn the_table_asks_the_mac_driver_for_mesas_egl_unless_a_game_opts_out() -> TestResult {
        let table = table();
        let gl = rule(&table, GL_RULE)?;
        assert_eq!(gl.exe, ANY_EXE);
        assert_eq!(gl.opengl, Some(OpenGl::Egl));
        // It is a setting, not an env entry, so nothing a process starts
        // inherits it.
        assert_eq!(gl.env, []);
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.opengl, Some(OpenGl::Egl));
        assert_eq!(game.set_by("opengl"), Some(GL_RULE));
        // A rule for one game gives it Apple's OpenGL and changes nothing else
        // for it; another process keeps the default.
        let table = with_overrides("name=my-game;exe=game.exe;opengl=cgl");
        let game = table.resolve("Game.exe", &VersionInfo::default());
        let mut expected = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(expected.opengl, Some(OpenGl::Egl));
        expected.opengl = Some(OpenGl::Cgl);
        set_by(&mut expected, "opengl", "my-game");
        expected.matched.push("my-game".to_string());
        assert_eq!(game, expected);
        // Dropping the rule leaves it unset: Apple's OpenGL everywhere.
        let table = with_overrides(&format!("name={GL_RULE};enabled=false"));
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.opengl, None);
        assert_eq!(game.matched, wildcards_without(GL_RULE));
        assert_eq!(game.env, wildcard_env());
        Ok(())
    }

    #[test]
    fn the_table_keeps_no_execute_on_unless_a_game_opts_out() -> TestResult {
        let table = table();
        let nx = rule(&table, NX_RULE)?;
        assert_eq!(nx.exe, ANY_EXE);
        assert_eq!(nx.force_nx, Some(true));
        // It is a setting, not an env entry, so nothing a process starts
        // inherits it.
        assert_eq!(nx.env, []);
        // Any process gets it, whatever its name or version resource.
        let game = table.resolve("Some Game.exe", &VersionInfo::default());
        assert_eq!(game.force_nx, Some(true));
        assert_eq!(game.set_by("force_nx"), Some(NX_RULE));
        assert_eq!(game.env, wildcard_env());
        // A rule for one game opts that game out and nothing else changes for
        // it; another process keeps the default.
        let table = with_overrides("name=my-game;exe=game.exe;force_nx=false");
        let game = table.resolve("Game.exe", &VersionInfo::default());
        let mut expected = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(expected.force_nx, Some(true));
        expected.force_nx = Some(false);
        set_by(&mut expected, "force_nx", "my-game");
        expected.matched.push("my-game".to_string());
        assert_eq!(game, expected);
        // Dropping the rule leaves it unset: Wine's behaviour everywhere.
        let table = with_overrides(&format!("name={NX_RULE};enabled=false"));
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.force_nx, None);
        assert_eq!(game.matched, wildcards_without(NX_RULE));
        assert_eq!(game.env, wildcard_env());
        Ok(())
    }

    #[test]
    fn wined3d_on_vulkan_is_a_setting_for_one_game_and_never_a_default() {
        // wined3d on KosmicKrisp still crashes in too many places to be every
        // process's renderer, so no built-in rule sets the wined3d setting or
        // WINE_D3D_CONFIG. A game rule opts in, and only that game gets it.
        assert!(
            !table()
                .rules
                .iter()
                .any(|r| r.wined3d.is_some() || r.env.iter().any(|(n, _)| n == "WINE_D3D_CONFIG"))
        );
        let mut game = table().resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.apply_wined3d(), None);
        assert_eq!(game.env, wildcard_env());
        let table = with_overrides("name=my-game;exe=game.exe;wined3d=vulkan");
        let mut game = table.resolve("Game.exe", &VersionInfo::default());
        assert_eq!(game.matched, wildcards_and("my-game"));
        assert_eq!(game.apply_wined3d(), Some(Wined3d::Vulkan));
        let vulkan = ("WINE_D3D_CONFIG".to_string(), "renderer=vulkan".to_string());
        assert_eq!(game.env, wildcard_env_and(&[vulkan]));
        let mut other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.apply_wined3d(), None);
        assert_eq!(other.env, wildcard_env());
    }

    #[test]
    fn the_table_leaves_vulkan_1_alone() {
        // The bundle ships Wine's Vulkan, so no built-in rule disables the
        // loader. --compatdb-only relies on this when it refuses a bundle
        // without Vulkan, where a game's own vulkan-1.dll would load.
        let table = table();
        assert!(
            !table
                .rules
                .iter()
                .any(|r| r.dll_overrides.iter().any(|o| o.contains("vulkan")))
        );
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.dll_overrides, wildcard_overrides());
    }

    #[test]
    fn the_builtin_table_pins_the_launcher_and_round_trips() -> TestResult {
        let table = table();
        let launcher = rule(&table, "rockstar-launcher")?;
        assert_eq!(launcher.dxgi, Some(Dxgi::Wined3d));
        assert_eq!(launcher.company.as_deref(), Some("Rockstar Games"));
        let gta = rule(&table, "gta-iv")?;
        assert!(
            gta.arguments
                .first()
                .is_some_and(|a| a.starts_with("-availablevidmem"))
        );
        // The D3D9 quirks are mtld3d's, not ours.
        assert_eq!(gta.env, []);

        // Which env entries are defaults does not travel; everything else
        // does.
        let (parsed, diagnostics) = Table::parse(&table.to_env_value());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let mut sent = table;
        for rule in &mut sent.rules {
            rule.env_defaults = 0;
        }
        assert_eq!(parsed, sent);
        Ok(())
    }

    #[test]
    fn the_builtin_table_pins_the_cef_helpers_to_dxmt() -> TestResult {
        let table = table();
        for name in ["steam-web-helper", "rockstar-social-club-ui"] {
            let helper = rule(&table, name)?;
            assert_eq!(helper.dxgi, Some(Dxgi::Dxmt), "{name}");
            assert_eq!(helper.arguments, vec!["--in-process-gpu".to_string()]);
        }
        Ok(())
    }

    #[test]
    fn only_the_cef_helpers_set_env_and_it_turns_the_metal_hud_off() -> TestResult {
        // Every rule that matches a launcher or a game must leave the
        // environment alone, or the games started from it would inherit the
        // entries. The wildcard env rules (`wildcard_env_rules`) are the
        // exceptions: they are meant for every process, so a child that
        // inherits them gets what it would get anyway.
        let hud_off = vec![("MTL_HUD_ENABLED".to_string(), "0".to_string())];
        let helpers = ["steam-web-helper", "rockstar-social-club-ui"];
        let table = table();
        let env_rules = wildcard_env_rules();
        for rule in &table.rules {
            if helpers.contains(&rule.name.as_str()) {
                assert_eq!(rule.env, hud_off, "{}", rule.name);
            } else if let Some((_, env)) = env_rules.iter().find(|(n, _)| *n == rule.name) {
                assert_eq!(rule.env, *env, "{}", rule.name);
            } else {
                assert_eq!(rule.env, [], "{}", rule.name);
            }
        }
        let hud_off = wildcard_env_and(&hud_off);
        assert_eq!(rule(&table, "rockstar-launcher")?.env, []);
        // The resolved environment of a helper is that entry after the
        // wildcard ones.
        let steam = VersionInfo {
            company: "Valve Corporation".into(),
            product: "Steam Client WebHelper".into(),
            original_filename: "steamwebhelper.exe".into(),
        };
        assert_eq!(table.resolve("steamwebhelper.exe", &steam).env, hud_off);
        let social = VersionInfo {
            company: "Take-Two Interactive Software, Inc.".into(),
            product: "Social Club UI".into(),
            original_filename: "SocialClubHelper.exe".into(),
        };
        assert_eq!(table.resolve("SocialClubHelper.exe", &social).env, hud_off);
        let launcher_version = VersionInfo {
            company: "Rockstar Games".into(),
            product: "Rockstar Games Launcher".into(),
            original_filename: "Launcher.exe".into(),
        };
        let resolution = table.resolve("Launcher.exe", &launcher_version);
        assert_eq!(resolution.matched, wildcards_and("rockstar-launcher"));
        assert_eq!(resolution.env, wildcard_env());
        Ok(())
    }

    #[test]
    fn the_crate_exports_the_names_of_this_table() {
        // A launcher tells built-in names apart through the table crate's
        // list, so it has to be this table's, in order.
        let names: Vec<String> = table().rules.into_iter().map(|r| r.name).collect();
        assert_eq!(names, compatdb_table::BUILTIN_RULE_NAMES);
        for name in &names {
            assert!(compatdb_table::is_builtin_rule_name(
                &name.to_ascii_uppercase()
            ));
        }
        assert!(!compatdb_table::is_builtin_rule_name("my-game"));
    }

    #[test]
    fn every_builtin_rule_has_a_unique_name_and_none_matches_a_bare_basename() {
        // A rule naming an executable must also pin its version resource, or
        // it would catch every program of that name. The `*` rules are a
        // different case: they are meant for every process, and several of
        // them are not duplicates of each other.
        let table = table();
        for rule in &table.rules {
            assert!(!rule.name.is_empty(), "{} has no name", rule.exe);
            assert!(!rule.exe.is_empty(), "{} has no exe", rule.name);
            assert!(
                !rule.is_unfingerprinted(),
                "{} matches on its executable alone",
                rule.name
            );
        }
        let mut names: Vec<&str> = table.rules.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate rule name");
        assert_eq!(table.duplicate_matchers(), []);
    }

    #[test]
    fn every_process_gets_the_wildcard_defaults() {
        let resolution = table().resolve("game.exe", &VersionInfo::default());
        assert_eq!(resolution.matched, wildcards());
        assert_eq!(resolution.dll_overrides, wildcard_overrides());
        assert_eq!(resolution.dpi_aware, Some(true));
        // No `*` rule forces large address awareness: the executable's flag
        // decides, as in upstream Wine.
        assert_eq!(resolution.large_address_aware, None);
        assert_eq!(resolution.set_by("large_address_aware"), None);
        assert_eq!(resolution.gcmouse, Some(true));
        assert_eq!(resolution.force_nx, Some(true));
        assert_eq!(resolution.opengl, Some(OpenGl::Egl));
        assert_eq!(resolution.env, wildcard_env());
        assert_eq!(resolution.dxgi, Some(Dxgi::Dxmt));
        assert_eq!(resolution.set_by("dxgi"), Some("dxmt"));
        assert_eq!(resolution.d3d9, Some(D3d9::Mtld3d));
        assert_eq!(resolution.set_by("d3d9"), Some("mtld3d"));
    }

    #[test]
    fn the_pins_for_one_program_beat_the_direct3d_rules() -> TestResult {
        // Every built-in rule for one program that names a tree keeps it
        // over the `*` rules.
        let table = table();
        let pins: Vec<&Rule> = table
            .rules
            .iter()
            .filter(|r| r.exe != ANY_EXE && (r.dxgi.is_some() || r.d3d9.is_some()))
            .collect();
        assert_ne!(pins.len(), 0);
        for pin in pins {
            let version = VersionInfo {
                company: pin.company.clone().unwrap_or_default(),
                product: pin.product.clone().unwrap_or_default(),
                original_filename: pin.original_filename.clone().unwrap_or_default(),
            };
            let resolution = table.resolve(&pin.exe, &version);
            if let Some(dxgi) = pin.dxgi {
                assert_eq!(resolution.dxgi, Some(dxgi), "{}", pin.name);
                assert_eq!(resolution.set_by("dxgi"), Some(pin.name.as_str()));
            }
            if let Some(d3d9) = pin.d3d9 {
                assert_eq!(resolution.d3d9, Some(d3d9), "{}", pin.name);
                assert_eq!(resolution.set_by("d3d9"), Some(pin.name.as_str()));
            }
        }
        assert_eq!(rule(&table, "rockstar-launcher")?.dxgi, Some(Dxgi::Wined3d));
        Ok(())
    }

    #[test]
    fn dropping_a_direct3d_rule_gives_every_process_wined3d() {
        // No opinion is upstream Wine's wined3d, for each family on its own.
        let table = with_overrides("name=dxmt;enabled=false");
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.dxgi, None);
        assert_eq!(game.d3d9, Some(D3d9::Mtld3d));
        let table = with_overrides("name=mtld3d;enabled=false");
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.dxgi, Some(Dxgi::Dxmt));
        assert_eq!(game.d3d9, None);
        // A rule for one game gives it wined3d and leaves the others alone.
        let table = with_overrides("name=my-game;exe=game.exe;dxgi=wined3d;d3d9=wined3d");
        let game = table.resolve("Game.exe", &VersionInfo::default());
        assert_eq!(game.dxgi, Some(Dxgi::Wined3d));
        assert_eq!(game.d3d9, Some(D3d9::Wined3d));
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.dxgi, Some(Dxgi::Dxmt));
    }

    #[test]
    fn a_game_rule_turns_gcmouse_off() {
        let table = with_overrides("name=my-game;exe=Game.exe;gcmouse=false");
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.gcmouse, Some(false));
        assert_eq!(game.matched, wildcards_and("my-game"));
        // Nothing else changes for that game.
        let mut expected = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(expected.gcmouse, Some(true));
        expected.gcmouse = Some(false);
        set_by(&mut expected, "gcmouse", "my-game");
        expected.matched.push("my-game".to_string());
        assert_eq!(game, expected);
    }

    #[test]
    fn a_launch_wide_rule_turns_gcmouse_off_and_a_game_rule_back_on() {
        let table = with_overrides(
            "name=global;exe=*;gcmouse=false\nname=my-game;exe=Game.exe;gcmouse=true",
        );
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.gcmouse, Some(false));
        assert_eq!(other.matched, wildcards_and("global"));
        let game = table.resolve("Game.exe", &VersionInfo::default());
        assert_eq!(game.gcmouse, Some(true));
    }

    #[test]
    fn disabling_the_gcmouse_rule_turns_it_off() {
        // ntdll then hands winemac.drv no value from compatdb, and the Mac
        // driver keeps upstream Wine's behaviour: no GameController mice.
        let table = with_overrides("name=gcmouse;enabled=false");
        let resolution = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(resolution.gcmouse, None);
        assert_eq!(resolution.matched, wildcards_without("gcmouse"));
        assert_eq!(resolution.force_nx, Some(true));
        assert_eq!(resolution.dpi_aware, Some(true));
    }

    #[test]
    fn a_game_rule_gives_one_game_4_gb() {
        let table = with_overrides("name=old;exe=old.exe;large_address_aware=true");
        let old = table.resolve("Old.exe", &VersionInfo::default());
        assert_eq!(old.large_address_aware, Some(true));
        assert_eq!(old.set_by("large_address_aware"), Some("old"));
        assert_eq!(old.matched, wildcards_and("old"));
        // Nothing else changes for that game.
        let mut expected = table.resolve("other.exe", &VersionInfo::default());
        expected.large_address_aware = Some(true);
        set_by(&mut expected, "large_address_aware", "old");
        expected.matched.push("old".to_string());
        assert_eq!(old, expected);
        // Another process keeps the executable's flag.
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.large_address_aware, None);
        // A launch-wide rule gives every process 4 GB, and a rule for one
        // game takes it back to the executable's flag.
        let table = with_overrides(
            "name=global;exe=*;large_address_aware=true\n\
             name=old;exe=old.exe;large_address_aware=false",
        );
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.large_address_aware, Some(true));
        assert_eq!(other.matched, wildcards_and("global"));
        let old = table.resolve("Old.exe", &VersionInfo::default());
        assert_eq!(old.large_address_aware, Some(false));
    }

    #[test]
    fn an_override_of_the_old_large_address_aware_rule_names_no_rule() {
        // The built-in `*` rule that gave every process 4 GB is gone. A
        // launcher that still drops or changes it by name gets a warning, and
        // nothing else changes.
        for text in [
            "name=large-address-aware;enabled=false",
            "name=large-address-aware;large_address_aware=false",
        ] {
            let mut table = table();
            let (over, diagnostics) = Table::parse(text);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            assert_eq!(
                table.overlay(over),
                ["override names no rule: large-address-aware"],
                "{text}"
            );
            assert_eq!(table, super::table(), "{text}");
        }
        assert!(!compatdb_table::is_builtin_rule_name("large-address-aware"));
    }

    /// The version resource of World of Warcraft's `WoW.exe`, as compatdb.so
    /// reads it from the shipped 1.12.1 and 3.3.5a clients, the patched
    /// 1.12.1 client of the Turtle server included: the three strings are the
    /// same in all of them.
    fn wow_version() -> VersionInfo {
        VersionInfo {
            company: "Blizzard Entertainment".into(),
            product: "World of Warcraft".into(),
            original_filename: "WoW.exe".into(),
        }
    }

    #[test]
    fn world_of_warcraft_gets_4_gb_and_nothing_else_changes() {
        let table = table();
        let mut expected = table.resolve("game.exe", &VersionInfo::default());
        expected.large_address_aware = Some(true);
        set_by(&mut expected, "large_address_aware", "world-of-warcraft");
        expected.matched.push("world-of-warcraft".to_string());
        assert_eq!(table.resolve("WoW.exe", &wow_version()), expected);
        assert_eq!(expected.matched, wildcards_and("world-of-warcraft"));
        // The 3.3.5a client spells its file Wow.exe; the basename matches
        // case-insensitively.
        assert_eq!(table.resolve("Wow.exe", &wow_version()), expected);
        // A WoW.exe without that resource or from another company, and an
        // executable of another name with it, keep the executable's flag.
        let other = VersionInfo {
            company: "Some Vendor".into(),
            ..wow_version()
        };
        for (exe, version) in [
            ("WoW.exe", VersionInfo::default()),
            ("WoW.exe", other),
            ("WowError.exe", wow_version()),
        ] {
            let resolution = table.resolve(exe, &version);
            assert_eq!(resolution.large_address_aware, None, "{exe}");
            assert_eq!(resolution.matched, wildcards(), "{exe}");
        }
    }

    #[test]
    fn disabling_dpi_aware_leaves_the_awareness_to_wine() {
        let table = with_overrides("name=dpi-aware;enabled=false");
        let resolution = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(resolution.dpi_aware, None);
        assert_eq!(resolution.matched, wildcards_without("dpi-aware"));
    }

    #[test]
    fn a_launch_wide_rule_beats_the_builtin_dpi_default() {
        // Both are plain `*` rules, so specificity does not separate them.
        // The launcher's rule has a new name, so the overlay appends it after
        // the built-ins, and the stable sort in `resolve` keeps that order.
        let table = with_overrides("name=global;exe=*;dpi_aware=false");
        let resolution = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(resolution.dpi_aware, Some(false));
        assert_eq!(resolution.matched, wildcards_and("global"));
    }

    #[test]
    fn a_rule_for_one_program_makes_it_unaware() {
        let table = with_overrides("name=old;exe=old.exe;dpi_aware=false");
        assert_eq!(
            table.resolve("Old.exe", &VersionInfo::default()).dpi_aware,
            Some(false)
        );
        assert_eq!(
            table
                .resolve("other.exe", &VersionInfo::default())
                .dpi_aware,
            Some(true)
        );
    }

    #[test]
    fn a_launch_wide_rule_re_enables_mono_after_the_builtins() {
        // What a launcher sends for a top-level `dll_overrides`: its own `*`
        // rule. Within the wildcard tier the rules fold in table order, which
        // puts the built-ins first, and ntdll keeps the last entry for a
        // module, so `mscoree=b` wins while mshtml stays disabled.
        let table = with_overrides("name=global;exe=*;dll_overrides=mscoree=b");
        let resolution = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(
            resolution.dll_overrides,
            vec!["mscoree,mshtml=".to_string(), "mscoree=b".to_string()]
        );
        assert_eq!(resolution.matched, wildcards_and("global"));
        assert_eq!(table.duplicate_matchers(), []);
    }

    #[test]
    fn disabling_no_mono_gecko_drops_its_override() {
        let table = with_overrides("name=no-mono-gecko;enabled=false");
        assert_eq!(
            table
                .resolve("game.exe", &VersionInfo::default())
                .dll_overrides,
            [] as [String; 0]
        );
    }

    #[test]
    fn a_game_override_merges_into_the_builtin_it_names() -> TestResult {
        let table = with_overrides("name=rockstar-launcher;dxgi=dxmt");
        let launcher = rule(&table, "rockstar-launcher")?;
        assert_eq!(launcher.dxgi, Some(Dxgi::Dxmt));
        // The pin survives the override.
        assert_eq!(launcher.company.as_deref(), Some("Rockstar Games"));
        assert_eq!(launcher.exe, "Launcher.exe");
        Ok(())
    }

    #[test]
    fn the_steam_helper_rule_matches_the_real_binarys_resource() {
        // Guards the fingerprints against a typo: these are the strings read
        // out of the shipped exes.
        let steam = VersionInfo {
            company: "Valve Corporation".into(),
            product: "Steam Client WebHelper".into(),
            original_filename: "steamwebhelper.exe".into(),
        };
        let social = VersionInfo {
            company: "Take-Two Interactive Software, Inc.".into(),
            product: "Social Club UI".into(),
            original_filename: "SocialClubHelper.exe".into(),
        };
        let gta = VersionInfo {
            company: "Rockstar Games".into(),
            product: "Grand Theft Auto IV".into(),
            original_filename: "GTAIV.exe".into(),
        };
        // NosTale's client names its company and nothing else.
        let nostale = VersionInfo {
            company: "Entwell".into(),
            ..VersionInfo::default()
        };
        let aoe2 = aoe2_hd_version();
        let table = table();
        assert_eq!(
            table.resolve("steamwebhelper.exe", &steam).matched,
            wildcards_and("steam-web-helper")
        );
        assert_eq!(
            table.resolve("AoK HD.exe", &aoe2).matched,
            wildcards_and("aoe2-hd")
        );
        assert_eq!(
            table.resolve("SocialClubHelper.exe", &social).matched,
            wildcards_and("rockstar-social-club-ui")
        );
        assert_eq!(
            table.resolve("GTAIV.exe", &gta).matched,
            wildcards_and("gta-iv")
        );
        assert_eq!(
            table.resolve("NostaleClientX.exe", &nostale).matched,
            wildcards_and("nostale")
        );
        assert_eq!(
            table.resolve("WoW.exe", &wow_version()).matched,
            wildcards_and("world-of-warcraft")
        );
        // An unrelated program of the same name gets only the `*` rules.
        for exe in [
            "steamwebhelper.exe",
            "GTAIV.exe",
            "NostaleClientX.exe",
            "AoK HD.exe",
            "WoW.exe",
        ] {
            assert_eq!(
                table.resolve(exe, &VersionInfo::default()).matched,
                wildcards(),
                "{exe}"
            );
        }
    }

    #[test]
    fn the_nostale_client_is_dpi_unaware_and_nothing_else_changes() {
        // The fingerprinted rule beats the `dpi-aware` default for the client
        // alone; every other setting stays the wildcards'.
        let nostale = VersionInfo {
            company: "Entwell".into(),
            ..VersionInfo::default()
        };
        let table = table();
        let mut expected = table.resolve("game.exe", &VersionInfo::default());
        expected.dpi_aware = Some(false);
        set_by(&mut expected, "dpi_aware", "nostale");
        expected.matched.push("nostale".to_string());
        assert_eq!(table.resolve("NostaleClientX.exe", &nostale), expected);
        assert_eq!(expected.matched, wildcards_and("nostale"));
        assert_eq!(
            table
                .resolve("NostaleClientX.exe", &VersionInfo::default())
                .dpi_aware,
            Some(true)
        );
    }

    /// The version resource of Age of Empires II HD's `AoK HD.exe`, as
    /// compatdb.so reads it from the shipped exe.
    fn aoe2_hd_version() -> VersionInfo {
        VersionInfo {
            company: "Microsoft Corporation".into(),
            product: "age2_x1".into(),
            original_filename: "age2_x1.exe".into(),
        }
    }

    #[test]
    fn age_of_empires_ii_hd_is_dpi_unaware_and_nothing_else_changes() {
        // The fingerprinted rule beats the `dpi-aware` default for the game
        // alone; every other setting stays the wildcards'.
        let table = table();
        let mut expected = table.resolve("game.exe", &VersionInfo::default());
        expected.dpi_aware = Some(false);
        set_by(&mut expected, "dpi_aware", "aoe2-hd");
        expected.matched.push("aoe2-hd".to_string());
        assert_eq!(table.resolve("AoK HD.exe", &aoe2_hd_version()), expected);
        assert_eq!(expected.matched, wildcards_and("aoe2-hd"));
        // Only the company is pinned, so a build with another product name
        // still matches.
        let newer = VersionInfo {
            product: "Age of Empires II HD".into(),
            ..aoe2_hd_version()
        };
        assert_eq!(table.resolve("AoK HD.exe", &newer).dpi_aware, Some(false));
        // An executable of another name with the same version resource keeps
        // the default, and so does one of the same name from another company.
        assert_eq!(
            table.resolve("age2_x1.exe", &aoe2_hd_version()).dpi_aware,
            Some(true)
        );
        let other = VersionInfo {
            company: "Some Vendor".into(),
            ..aoe2_hd_version()
        };
        let resolution = table.resolve("AoK HD.exe", &other);
        assert_eq!(resolution.dpi_aware, Some(true));
        assert_eq!(resolution.matched, wildcards());
    }
}
