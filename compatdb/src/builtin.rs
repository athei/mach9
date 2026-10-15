//! The built-in rules, compiled into `compatdb.so`.
//!
//! This is static data the library owns, so it never travels through the
//! environment. Only overrides are serialized into `WINE_COMPATDB`; the
//! library overlays them onto these rules by name.
//!
//! Every rule that names an executable also carries a version fingerprint, so
//! none of them can catch an unrelated program that happens to share a file
//! name. The exceptions are `no-vulkan`, `dpi-aware`, `large-address-aware`,
//! `x87-sidecar` and `no-mono-gecko`, which match every process on purpose:
//! each is a default that a more specific rule can override and that can be
//! dropped by name.
//!
//! The rules differ by [`Host`] only where a rule cannot apply to that Wine:
//! the arm64 Wine has no `x87-sidecar` rule, since it runs no Rosetta.

use compatdb_table::{ANY_EXE, Dxgi, Host, Rule, Table};

/// The rules the bundle for this library's own host ships with.
pub fn table() -> Table {
    table_for(Host::current())
}

/// The rules the bundle for `host` ships with.
pub fn table_for(host: Host) -> Table {
    let mut rules = vec![
        // The bundle ships no Vulkan: Wine is built --without-vulkan and
        // bundle-wine.sh deletes vulkan-1.dll and winevulkan. A Vulkan
        // loader a game ships next to its exe can therefore only fail, so
        // disable vulkan-1 in every process: the loader then fails to
        // load, and a game that has another renderer uses it. A rule for
        // one game's exe that adds `vulkan-1=n` is folded after this one
        // and wins.
        Rule {
            name: "no-vulkan".into(),
            exe: ANY_EXE.into(),
            dll_overrides: vec!["vulkan-1=".into()],
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
        // Every 32-bit process gets the 4 GB address space, whether or not
        // its executable has the large-address-aware flag. Older games
        // that run out of their 2 GB long before they would on Windows,
        // because Wine itself and the Direct3D layers take a share, get
        // the headroom. A rule for one game with `large_address_aware =
        // false` stops forcing it; an executable that has the flag keeps
        // it regardless. 64-bit processes are not affected.
        Rule {
            name: "large-address-aware".into(),
            exe: ANY_EXE.into(),
            large_address_aware: Some(true),
            ..Rule::default()
        },
    ];
    // Every i386 process under Rosetta starts with the x87sidecar attached,
    // which runs x87 floating-point code faster than Rosetta does on its
    // own. ntdll asks compatdb_query_x87 before it starts the process, so a
    // rule for one game with `x87_sidecar = false` keeps the sidecar away
    // from it. The arm64 Wine runs no Rosetta and its library has no such
    // query, so it gets no rule.
    if host == Host::X86_64 {
        rules.push(Rule {
            name: "x87-sidecar".into(),
            exe: ANY_EXE.into(),
            x87_sidecar: Some(true),
            ..Rule::default()
        });
    }
    rules.extend([
        // Keeps Wine from prompting to install Mono and Gecko, in
        // wineboot and in any process that loads either. It is a separate
        // rule so disabling `no-vulkan` does not bring the prompts back.
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
        // The Rockstar Games Launcher needs a real D3D10.1 device, which
        // neither 64-bit default (Apple's D3DMetal, or DXMT in the arm64
        // Wine) provides; wined3d does. Matched by its version resource,
        // not its path, so it works wherever it is installed and never
        // hits another game's Launcher.exe.
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
        // rendering stays on and goes through DXMT, pinned off the 64-bit
        // default (D3DMetal) because the Steam client runs better on
        // DXMT; the dxgi pin keeps it there even when a wildcard rule
        // selects another tree. The Metal HUD is off for the same reason
        // as on the Social Club helper; steam.exe, which starts the games,
        // has no rule.
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
    ]);
    Table { rules }
}

#[cfg(test)]
mod tests {
    use compatdb_table::{HEADER, VersionInfo};

    use super::*;

    /// Both hosts, so every test covers both tables on one machine.
    const HOSTS: [Host; 2] = [Host::X86_64, Host::Aarch64];

    /// What a test returns, so a missing value fails it through `?` rather
    /// than a panic in the test body.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// The rule of `table` named `name`.
    fn rule<'a>(table: &'a Table, name: &str) -> Result<&'a Rule, String> {
        table
            .rules
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| format!("no rule {name}"))
    }

    /// The names of the rules that match every process on `host`, in table
    /// order.
    fn wildcards(host: Host) -> Vec<String> {
        let mut names = vec![
            "no-vulkan".to_string(),
            "dpi-aware".to_string(),
            "large-address-aware".to_string(),
        ];
        if host == Host::X86_64 {
            names.push("x87-sidecar".to_string());
        }
        names.push("no-mono-gecko".to_string());
        names
    }

    /// `wildcards(host)` without the rule of the given name.
    fn wildcards_without(host: Host, name: &str) -> Vec<String> {
        wildcards(host).into_iter().filter(|n| n != name).collect()
    }

    /// `wildcards(host)` followed by one more specific rule.
    fn wildcards_and(host: Host, name: &str) -> Vec<String> {
        let mut names = wildcards(host);
        names.push(name.to_string());
        names
    }

    /// What the built-in rules say about the x87sidecar on `host`.
    fn x87_default(host: Host) -> Option<bool> {
        (host == Host::X86_64).then_some(true)
    }

    /// The built-in table for `host` with the rules in `text` (after the
    /// header) laid on top, which must parse cleanly and all find a home.
    fn with_overrides(host: Host, text: &str) -> Table {
        let mut table = table_for(host);
        let (over, diagnostics) = Table::parse(&format!("{HEADER}\n{text}"));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(table.overlay(over), [] as [String; 0], "{host:?}");
        table
    }

    #[test]
    fn the_table_is_the_one_for_the_compile_target() {
        assert_eq!(table(), table_for(Host::current()));
    }

    #[test]
    fn only_the_x86_64_table_has_the_x87_sidecar_rule() {
        let has = |host| {
            table_for(host)
                .rules
                .iter()
                .any(|r| r.name == "x87-sidecar" || r.x87_sidecar.is_some())
        };
        assert!(has(Host::X86_64));
        assert!(!has(Host::Aarch64));
        // Apart from that rule, the two tables are the same.
        let mut intel = table_for(Host::X86_64);
        intel.rules.retain(|r| r.name != "x87-sidecar");
        assert_eq!(intel, table_for(Host::Aarch64));
    }

    #[test]
    fn the_arm64_table_accepts_an_x87_sidecar_override() {
        // A launcher may send the same WINE_COMPATDB to either Wine. The key
        // parses, so the record is kept; the arm64 library just never asks
        // for it.
        let arm = Host::Aarch64;
        let table = with_overrides(
            arm,
            "name=old;exe=old.exe;dpi_aware=false;x87_sidecar=false",
        );
        let old = table.resolve("Old.exe", &VersionInfo::default());
        assert_eq!(old.dpi_aware, Some(false));
        assert_eq!(old.x87_sidecar, Some(false));
        assert_eq!(old.matched, wildcards_and(arm, "old"));
        // Disabling the x86_64 rule by name finds nothing to drop there,
        // which the library reports as an override that names no rule.
        let mut table = table_for(arm);
        let (over, _) = Table::parse(&format!("{HEADER}\nname=x87-sidecar;enabled=false"));
        assert_eq!(table.overlay(over), ["x87-sidecar".to_string()]);
        assert_eq!(table, table_for(arm));
    }

    #[test]
    fn the_builtin_table_pins_the_launcher_and_round_trips() -> TestResult {
        for host in HOSTS {
            let table = table_for(host);
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

            let (parsed, diagnostics) = Table::parse(&table.to_env_value());
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            assert_eq!(parsed, table);
        }
        Ok(())
    }

    #[test]
    fn the_builtin_table_pins_the_cef_helpers_to_dxmt() -> TestResult {
        for host in HOSTS {
            let table = table_for(host);
            for name in ["steam-web-helper", "rockstar-social-club-ui"] {
                let helper = rule(&table, name)?;
                assert_eq!(helper.dxgi, Some(Dxgi::Dxmt), "{name}");
                assert_eq!(helper.arguments, vec!["--in-process-gpu".to_string()]);
            }
        }
        Ok(())
    }

    #[test]
    fn only_the_cef_helpers_set_env_and_it_turns_the_metal_hud_off() -> TestResult {
        // Every rule that matches a launcher or a game, or every process,
        // must leave the environment alone, or the games started from it
        // would inherit the entries.
        let hud_off = vec![("MTL_HUD_ENABLED".to_string(), "0".to_string())];
        let helpers = ["steam-web-helper", "rockstar-social-club-ui"];
        for host in HOSTS {
            let table = table_for(host);
            for rule in &table.rules {
                if helpers.contains(&rule.name.as_str()) {
                    assert_eq!(rule.env, hud_off, "{}", rule.name);
                } else {
                    assert_eq!(rule.env, [], "{}", rule.name);
                }
            }
            assert_eq!(rule(&table, "rockstar-launcher")?.env, []);
            // The resolved environment of a helper is that entry alone.
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
            assert_eq!(resolution.matched, wildcards_and(host, "rockstar-launcher"));
            assert_eq!(resolution.env, []);
        }
        Ok(())
    }

    #[test]
    fn every_builtin_rule_has_a_unique_name_and_none_matches_a_bare_basename() {
        // A rule naming an executable must also pin its version resource, or
        // it would catch every program of that name. The `*` rules are a
        // different case: they are meant for every process, and several of
        // them are not duplicates of each other.
        for host in HOSTS {
            let table = table_for(host);
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
    }

    #[test]
    fn every_process_gets_the_wildcard_defaults() {
        for host in HOSTS {
            let resolution = table_for(host).resolve("game.exe", &VersionInfo::default());
            assert_eq!(resolution.matched, wildcards(host));
            assert_eq!(
                resolution.dll_overrides,
                vec!["vulkan-1=".to_string(), "mscoree,mshtml=".to_string()]
            );
            assert_eq!(resolution.dpi_aware, Some(true));
            assert_eq!(resolution.large_address_aware, Some(true));
            assert_eq!(resolution.x87_sidecar, x87_default(host), "{host:?}");
        }
    }

    #[test]
    fn a_game_rule_turns_large_address_awareness_and_the_x87_sidecar_off() {
        for host in HOSTS {
            let table = with_overrides(
                host,
                "name=old;exe=old.exe;large_address_aware=false;x87_sidecar=false",
            );
            let old = table.resolve("Old.exe", &VersionInfo::default());
            assert_eq!(old.large_address_aware, Some(false));
            assert_eq!(old.x87_sidecar, Some(false));
            assert_eq!(old.matched, wildcards_and(host, "old"));
            // Another process keeps both defaults.
            let other = table.resolve("other.exe", &VersionInfo::default());
            assert_eq!(other.large_address_aware, Some(true));
            assert_eq!(other.x87_sidecar, x87_default(host));
            // The plain wildcard rules still stack without being reported.
            assert_eq!(table.duplicate_matchers(), []);
        }
    }

    #[test]
    fn disabling_the_laa_and_x87_rules_leaves_no_opinion() {
        let table = with_overrides(
            Host::X86_64,
            "name=large-address-aware;enabled=false\nname=x87-sidecar;enabled=false",
        );
        let resolution = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(resolution.large_address_aware, None);
        assert_eq!(resolution.x87_sidecar, None);
        assert_eq!(resolution.dpi_aware, Some(true));
        // The arm64 table has no x87 rule to drop.
        let table = with_overrides(Host::Aarch64, "name=large-address-aware;enabled=false");
        let resolution = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(resolution.large_address_aware, None);
        assert_eq!(resolution.x87_sidecar, None);
        assert_eq!(resolution.dpi_aware, Some(true));
    }

    #[test]
    fn disabling_dpi_aware_leaves_the_awareness_to_wine() {
        for host in HOSTS {
            let table = with_overrides(host, "name=dpi-aware;enabled=false");
            let resolution = table.resolve("game.exe", &VersionInfo::default());
            assert_eq!(resolution.dpi_aware, None);
            assert_eq!(resolution.matched, wildcards_without(host, "dpi-aware"));
        }
    }

    #[test]
    fn a_launch_wide_rule_beats_the_builtin_dpi_default() {
        // Both are plain `*` rules, so specificity does not separate them.
        // The launcher's rule has a new name, so the overlay appends it after
        // the built-ins, and the stable sort in `resolve` keeps that order.
        for host in HOSTS {
            let table = with_overrides(host, "name=global;exe=*;dpi_aware=false");
            let resolution = table.resolve("game.exe", &VersionInfo::default());
            assert_eq!(resolution.dpi_aware, Some(false));
            assert_eq!(resolution.matched, wildcards_and(host, "global"));
        }
    }

    #[test]
    fn a_rule_for_one_program_makes_it_unaware() {
        for host in HOSTS {
            let table = with_overrides(host, "name=old;exe=old.exe;dpi_aware=false");
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
    }

    #[test]
    fn a_launch_wide_rule_re_enables_mono_after_the_builtins() {
        // What a launcher sends for a top-level `dll_overrides`: its own `*`
        // rule. Within the wildcard tier the rules fold in table order, which
        // puts the built-ins first, and ntdll keeps the last entry for a
        // module, so `mscoree=b` wins while mshtml stays disabled.
        for host in HOSTS {
            let table = with_overrides(host, "name=global;exe=*;dll_overrides=mscoree=b");
            let resolution = table.resolve("game.exe", &VersionInfo::default());
            assert_eq!(
                resolution.dll_overrides,
                vec![
                    "vulkan-1=".to_string(),
                    "mscoree,mshtml=".to_string(),
                    "mscoree=b".to_string()
                ]
            );
            assert_eq!(resolution.matched, wildcards_and(host, "global"));
            assert_eq!(table.duplicate_matchers(), []);
        }
    }

    #[test]
    fn disabling_no_mono_gecko_drops_only_its_override() {
        for host in HOSTS {
            let table = with_overrides(host, "name=no-mono-gecko;enabled=false");
            assert_eq!(
                table
                    .resolve("game.exe", &VersionInfo::default())
                    .dll_overrides,
                vec!["vulkan-1=".to_string()]
            );
        }
    }

    #[test]
    fn a_game_rule_re_enables_vulkan_after_the_builtin_disables_it() {
        // ntdll lets the last override for a module win, so the game's entry
        // has to come after the built-in one. That holds wherever the game's
        // rule is declared, because the `*` rules are folded first.
        for host in HOSTS {
            let table = with_overrides(host, "name=my-game;exe=game.exe;dll_overrides=vulkan-1=n");
            let resolution = table.resolve("Game.exe", &VersionInfo::default());
            assert_eq!(
                resolution.dll_overrides,
                vec![
                    "vulkan-1=".to_string(),
                    "mscoree,mshtml=".to_string(),
                    "vulkan-1=n".to_string()
                ]
            );
            // Another process still has it disabled.
            assert_eq!(
                table
                    .resolve("other.exe", &VersionInfo::default())
                    .dll_overrides,
                vec!["vulkan-1=".to_string(), "mscoree,mshtml=".to_string()]
            );
        }
    }

    #[test]
    fn disabling_no_vulkan_drops_the_override() {
        for host in HOSTS {
            let table = with_overrides(host, "name=no-vulkan;enabled=false");
            let resolution = table.resolve("game.exe", &VersionInfo::default());
            assert_eq!(resolution.matched, wildcards_without(host, "no-vulkan"));
            // The Mono and Gecko prompts stay off.
            assert_eq!(
                resolution.dll_overrides,
                vec!["mscoree,mshtml=".to_string()]
            );
        }
    }

    #[test]
    fn a_game_override_merges_into_the_builtin_it_names() -> TestResult {
        for host in HOSTS {
            let table = with_overrides(host, "name=rockstar-launcher;dxgi=dxmt");
            let launcher = rule(&table, "rockstar-launcher")?;
            assert_eq!(launcher.dxgi, Some(Dxgi::Dxmt));
            // The pin survives the override.
            assert_eq!(launcher.company.as_deref(), Some("Rockstar Games"));
            assert_eq!(launcher.exe, "Launcher.exe");
        }
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
        for host in HOSTS {
            let table = table_for(host);
            assert_eq!(
                table.resolve("steamwebhelper.exe", &steam).matched,
                wildcards_and(host, "steam-web-helper")
            );
            assert_eq!(
                table.resolve("SocialClubHelper.exe", &social).matched,
                wildcards_and(host, "rockstar-social-club-ui")
            );
            assert_eq!(
                table.resolve("GTAIV.exe", &gta).matched,
                wildcards_and(host, "gta-iv")
            );
            assert_eq!(
                table.resolve("NostaleClientX.exe", &nostale).matched,
                wildcards_and(host, "nostale")
            );
            // An unrelated program of the same name gets only the `*` rules.
            let only_wildcard = wildcards(host);
            for exe in ["steamwebhelper.exe", "GTAIV.exe", "NostaleClientX.exe"] {
                assert_eq!(
                    table.resolve(exe, &VersionInfo::default()).matched,
                    only_wildcard,
                    "{exe}"
                );
            }
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
        for host in HOSTS {
            let table = table_for(host);
            let mut expected = table.resolve("game.exe", &VersionInfo::default());
            expected.dpi_aware = Some(false);
            expected.matched.push("nostale".to_string());
            assert_eq!(table.resolve("NostaleClientX.exe", &nostale), expected);
            assert_eq!(expected.matched, wildcards_and(host, "nostale"));
            assert_eq!(
                table
                    .resolve("NostaleClientX.exe", &VersionInfo::default())
                    .dpi_aware,
                Some(true)
            );
        }
    }
}
