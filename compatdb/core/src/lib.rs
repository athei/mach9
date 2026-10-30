//! The per-process rules of this Wine's `compatdb.so`, and the wire format
//! a launcher hands them over in.
//!
//! `compatdb.so` is the unix library Wine's ntdll loads into every process
//! of the bundle before any PE code runs. It reads the process's image
//! name, version resource and Steam app id, takes the built-in rules of
//! [`builtin`], appends the rules a launcher passed in the environment variable
//! [`ENV_VAR`] (`WINE_COMPATDB`) or in the file [`FILE_ENV_VAR`] names,
//! folds every rule that matches the process into one [`Resolution`] and
//! applies it: the Direct3D trees to load, the renderer of wined3d, which
//! OpenGL the Mac driver gives, DLL load-order overrides, the DPI awareness,
//! large address awareness, whether the Mac driver reads raw mouse input
//! from `GameController`, whether no-execute stays on, whether the Metal
//! performance HUD shows, command-line arguments and environment entries.
//! It logs every value with the rule it came from: the lines on Wine's
//! stderr that start with `compatdb:`, which
//! [docs/compatdb.md](https://github.com/athei/mach9/blob/main/docs/compatdb.md#logging-and-verification)
//! describes, show a launcher author what a process got and why.
//!
//! This crate is the only place that knows the rules and their format. A
//! launcher uses it to build its rules ([`Rule`], [`Table`]), to write the
//! value of [`ENV_VAR`] (`Table`'s `Display`) and to show what a program
//! would get ([`Table::preview`]). `compatdb.so` uses the same crate to
//! read the value ([`Table::parse_lenient`]) and resolve it
//! ([`Table::resolve`]); the [`runtime`] module holds what only it needs.
//!
//! Everything here is pure: it works on `&str`, `&[u16]` (UTF-16 as Wine
//! hands it over) and owned values, with no unsafe and no I/O, so the
//! decision logic is unit-tested on the host and the `.so` itself only has
//! to move bytes in and out of the process.
//!
//! # Quick start
//!
//! A launcher builds one rule for every process from its launch-wide
//! settings and one per program it knows something about, writes the
//! table and starts Wine with it in [`ENV_VAR`]. The whole process tree
//! inherits the variable, so a rule for a program the game starts reaches
//! it too. The value is fixed when the tree starts, so a game started by a
//! program that is already running, such as a store client, gets that
//! program's value. For rules that change while a tree runs, see
//! [`FILE_ENV_VAR`] under [Format](#format).
//!
//! The settings layer fills a rule from text without the launcher knowing
//! any key: [`Key::from_setting`] names the key, [`Key::kind`] says whether
//! it holds one value, a list or environment entries, and [`Rule::set`],
//! [`Rule::set_list`] or [`Rule::set_env`] take the value. Every error is a
//! [`FieldError`] to print as it is, and [`FieldError::severity`] says
//! whether to warn and go on or to refuse the settings.
//!
//! ```
//! use std::process::Command;
//!
//! use compatdb_core::{
//!     Argument, Dxgi, ENV_VAR, Exe, Fingerprint, Key, Rule, Severity, Table, VersionInfo,
//! };
//!
//! // The launch-wide settings, as the launcher's own config holds them.
//! let mut every = Rule::new(Exe::Any);
//! for (name, value) in [("dpi_aware", "false"), ("GCMouse", "Default")] {
//!     every.set(Key::from_setting(name)?, value)?;
//! }
//!
//! // A rule for one game, from typed parts.
//! let game = Rule::new(Exe::new("Game.exe")?)
//!     .with_company(Fingerprint::new("Some Vendor")?)
//!     .with_dxgi(Dxgi::Wined3d)
//!     .with_argument(Argument::new("-windowed")?);
//!
//! // A retired value is a warning: the rule stays as it was.
//! let mut old = Rule::new(Exe::new("Old.exe")?);
//! let error = old.set(Key::Dxgi, "gptk").err().ok_or("gptk was taken")?;
//! assert_eq!(error.severity(), Severity::Warning);
//! assert!(old.sets_nothing());
//!
//! let mut table = Table::new();
//! table.push(every);
//! table.push(game);
//! let value = table.to_string();
//! assert_eq!(
//!     value,
//!     "exe=*;dpi_aware=false;gcmouse=default\n\
//!      exe=Game.exe;company=Some Vendor;dxgi=wined3d;arguments=-windowed"
//! );
//! let mut wine = Command::new("bin/wine");
//! wine.env(ENV_VAR, &value);
//!
//! // What that game would get, the built-in rules included.
//! let version = VersionInfo {
//!     company: "Some Vendor Ltd.".into(),
//!     ..VersionInfo::default()
//! };
//! let resolution = table.preview("Game.exe", &version);
//! assert_eq!(resolution.dxgi(), Some(Dxgi::Wined3d));
//! assert_eq!(resolution.dpi_aware(), Some(false));
//! // `default` hands gcmouse back to upstream Wine.
//! assert_eq!(resolution.gcmouse(), None);
//! assert_eq!(resolution.arguments, ["-windowed"]);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Format
//!
//! The value of [`ENV_VAR`] is one record per line, each a rule:
//!
//! ```text
//! exe=Game.exe;company=Some Vendor;d3d9=wined3d;dpi_aware=false;env=VKD3D_CONFIG=a%3Bb
//! exe=OldGame.exe;large_address_aware=true
//! exe=*;gcmouse=false
//! ```
//!
//! - A line ends at LF, and a CR before the LF is dropped. A line of
//!   whitespace alone is skipped. Lines are counted from 1, blank lines
//!   included; that number is the rule's [`Source::Line`], and the line
//!   of each [`Diagnostic`].
//! - A record is fields joined by `;`. An empty field is skipped.
//! - A field is `key=value`, and the key ends at the first `=`. The keys
//!   are those of [`Key`].
//! - Every record needs an `exe`. The other matchers, `company`, `product`
//!   and `original_filename`, are optional (see [Matching](#matching)).
//!   A matcher or a scalar setting holds one value, and a list takes one
//!   entry per field, in order.
//! - A scalar setting takes one of the spellings of [`Key::values`]: one
//!   of its words, or `default`, which resets it ([`Setting::Reset`]).
//! - `arguments_override` with an empty value sets the override without
//!   adding an entry, so `arguments_override=` alone clears the arguments.
//!   An empty entry of `arguments` or `debug_channels` is refused.
//! - An `env` field is `env=NAME=value`. The name ends at the first `=`,
//!   so a name cannot hold one ([`EnvName`]); the value may. An empty
//!   value, or none, removes the variable.
//! - What each list entry and matcher may hold is the type of its part:
//!   [`Exe`], [`Fingerprint`], [`DllOverride`] (needs a `=`), [`Argument`],
//!   [`DebugChannel`] (no `,`) and [`EnvName`]. None of them holds a NUL.
//!
//! Escaping. In a value, `Display` writes `%`, `;` and every ASCII control
//! character (U+0000 to U+001F and U+007F, CR and LF among them) as `%XX`,
//! the uppercase hex of the byte. Everything else, `=`, `,`, spaces and
//! non-ASCII included, is written as it is, in UTF-8. Reading decodes every
//! `%XX`, in either case, to its byte, and the bytes of a value must be
//! UTF-8 without a NUL: `%C3%A9` is `é`, and `%FF` or `%00` drops the
//! field. A `%` that two hex digits do not follow is kept as it is, with a
//! diagnostic.
//!
//! The case rule. Every word the format defines is read
//! ASCII-case-insensitively and written in lowercase: the keys, the words
//! of the scalar settings, `true`, `false`, `default`, the `steam:` of an
//! `exe` and the `v` of a legacy header. User data is kept byte for byte: the
//! `exe`, the fingerprint values, list entries and env names and values. So
//! `DXGI=WineD3D` reads as `dxgi=wined3d`, and `exe=STEAM:440` as
//! `exe=steam:440`, while `exe=Game.EXE` keeps its case (matching ignores it,
//! see [Matching](#matching)).
//!
//! There is no format version. A first line of `v=` and ASCII digits, the
//! header launchers wrote for an older format, is skipped without a
//! diagnostic; it still counts as a line. Compatibility is per field
//! instead: `compatdb.so` ignores a field whose key it does not know or
//! whose value its key does not take, with a warning, and applies the rest
//! of the record. A key added later is therefore only ever a setting: the
//! matcher keys are a closed set, since a library that ignored an unknown
//! matcher would apply the rule to more processes than it names. So a Steam
//! app is a value of `exe`, `steam:<appid>`, and no key of its own: a library
//! from before it reads the value as a basename, which no process has, and
//! the rule matches nothing, where `exe=*;steam_app=440` would apply to every
//! process. The `name` and `enabled` keys of the older format, where a line
//! changed or dropped a built-in rule by its name, are unknown keys like any
//! other, and a `compatdb.so` from before that change drops every line
//! without `name`, so a table in this format does nothing on an older bundle.
//!
//! `Display` writes the canonical text: one line per rule and no line
//! break after the last, the fields in the order of [`Key::ALL`], every
//! word in lowercase, and none of the built-in rules, which `compatdb.so`
//! has compiled in.
//!
//! [`FILE_ENV_VAR`] (`WINE_COMPATDB_FILE`) passes the same text in a file:
//! the variable holds the path and each process reads the file when it
//! starts, so a process started later in a running tree gets the rules the
//! file holds by then.
//!
//! - The file's text is what [`ENV_VAR`] would hold, and it is read, matched
//!   and folded the same way. Only `debug_channels` differs, see
//!   [Launcher responsibilities](#launcher-responsibilities). Its rules keep
//!   their labels in the log, `WINE_COMPATDB line <n>`.
//! - An empty [`FILE_ENV_VAR`] counts as unset. Set, the file is the only
//!   source of the caller's rules, and [`ENV_VAR`], if set too, is ignored
//!   with a warning.
//! - The path must be absolute and name a regular file within the size
//!   limit (`MAX_BYTES` in
//!   [`rules_file.rs`](https://github.com/athei/mach9/blob/main/compatdb/src/rules_file.rs)).
//!   Bytes that are not UTF-8 are replaced, as in the variable.
//! - A file that cannot be read gives the process the built-in rules alone,
//!   with a warning. [`ENV_VAR`] is not a fallback: it would hide a broken
//!   path behind rules that may be stale.
//!
//! ## Strict and lenient parsing
//!
//! [`Table::parse_lenient`] reads the value the way `compatdb.so` does,
//! which must never abort a process. Each problem is a [`Diagnostic`],
//! whose `Display` is the warning the library logs:
//!
//! - A field with an unknown key, a value its key or its part does not
//!   take, no `=`, escapes that are not UTF-8, or a NUL is left out, and
//!   the rest of its line still applies.
//! - A line without an `exe` is left out as a whole, since it matches no
//!   process.
//! - A second field for a matcher or a scalar setting replaces the first.
//! - A `%` that two hex digits do not follow is kept as it is.
//!
//! [`Table::parse_strict`], which is also `Table`'s `FromStr`, refuses
//! every problem lenient parsing reports, with a [`ParseError`] that lists
//! them. A launcher that writes the value with `Display` never needs to
//! read it; strict parsing is for text a person wrote.
//!
//! Two guarantees hold, and the [round-trip suite] checks them on
//! generated tables: a table built with [`Table::new`] and [`Table::push`]
//! reads back from its text as itself, and canonical text writes back byte
//! for byte.
//!
//! [round-trip suite]: https://github.com/athei/mach9/blob/main/compatdb/core/tests/roundtrip.rs
//!
//! ```
//! use compatdb_core::Table;
//!
//! let table: Table = "EXE=Game.exe;DPI_Aware=False;env=VKD3D_CONFIG=a%3Bb".parse()?;
//! let text = table.to_string();
//! assert_eq!(text, "exe=Game.exe;dpi_aware=false;env=VKD3D_CONFIG=a%3Bb");
//! assert_eq!(Table::parse_strict(&text)?, table);
//! assert_eq!(Table::parse_strict(&text)?.to_string(), text);
//!
//! // Lenient parsing keeps what it can read and says what it left out.
//! let (table, diagnostics) = Table::parse_lenient("exe=Game.exe;bogus=1;gcmouse=yes");
//! assert_eq!(table.rules().len(), 1);
//! let warnings: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
//! assert_eq!(
//!     warnings,
//!     [
//!         "WINE_COMPATDB line 1: ignoring unknown key \"bogus\"",
//!         "WINE_COMPATDB line 1: ignoring unknown gcmouse value \"yes\"",
//!     ]
//! );
//! assert!(Table::parse_strict("exe=Game.exe;bogus=1").is_err());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Matching
//!
//! A rule has no name: it is its matchers and its settings
//! ([`Rule::matches`]).
//!
//! - `exe` matches the image basename ASCII-case-insensitively, or every
//!   process when it is `*` ([`Exe::Any`]).
//! - `exe=steam:<appid>` ([`Exe::Steam`]) matches a process by its Steam app
//!   id instead, whatever its image name, for a launcher that drives
//!   `steam.exe -applaunch <appid>` and knows the id but not the executable.
//!   The id is the decimal value of `SteamAppId` in the process's environment
//!   block ([`runtime::steam_app`]). A value that is empty, 0 or no number is
//!   no id, so Steam's own processes, which have the variable empty, do not
//!   match. A game started outside Steam has no id unless whoever started it
//!   set the variable, so the rule does not match it, and neither does
//!   [`Table::preview`], which knows no id. [`Exe::new`] refuses a `steam:`
//!   that no id from 1 to [`u32::MAX`] follows.
//! - `company`, `product` and `original_filename` are optional
//!   ASCII-case-insensitive substrings of the image's version strings
//!   `CompanyName`, `ProductName` and `OriginalFilename` ([`VersionInfo`]).
//!   A string the image lacks reads as empty, so a rule that pins a value
//!   for it does not match. They combine with a Steam app id as with a
//!   basename: `exe=steam:440;product=Foo`.
//! - A rule whose `exe` matches but whose fingerprint does not does not
//!   apply; nothing falls back to it, and the other rules that match still
//!   apply.
//! - [`Table::duplicate_matchers`] finds rules that match exactly the same
//!   processes, which in a hand-written table is a mistake.
//! - A process has its app id from its environment block, which it inherits,
//!   so a rule for an app id can match more than one program. [`Rule::set`]
//!   therefore refuses in such a rule what it refuses in a `*` rule
//!   ([`Key::allowed_on_any`]), and `debug_channels`, since `WINEDEBUG` scopes
//!   an entry by the image name ([`Table::winedebug`] leaves one out). Reading
//!   text refuses neither, [`Table::parse_strict`] included: both stay in the
//!   rule and its text, and the rule's `arguments_override` is ignored and its
//!   `debug_channels` are only logged.
//!
//! # Folding
//!
//! Every rule that matches a process applies; no rule removes or replaces
//! another. [`Table::resolve`] folds them in this order, and within each
//! group a rule without a fingerprint before one with, otherwise in table
//! order (line order for [`ENV_VAR`]):
//!
//! 1. the built-in `*` rules;
//! 2. the `*` rules of [`ENV_VAR`];
//! 3. the built-in rules that name an executable;
//! 4. the rules of [`ENV_VAR`] that name an executable.
//!
//! A rule for a Steam app id counts as naming an executable: it adds no group
//! to this order, and within one folds in table order with the rules for a
//! basename ([`Specificity::Exe`], [`Specificity::ExeWithFingerprint`]). So it
//! beats a built-in rule for a program in every process that holds the id,
//! as a rule of the launcher for that program does: `exe=steam:12210;dxgi=dxmt`
//! moves a Rockstar `Launcher.exe` that holds the id off the tree its built-in
//! rule pins, and `exe=steam:12210;metal_hud=true` undoes the
//! `MTL_HUD_ENABLED=0` of such a `SocialClubHelper.exe`. A fingerprint narrows
//! the rule to the game: `exe=steam:12210;product=Grand Theft Auto IV;dxgi=dxmt`.
//!
//! So a launch-wide `*` rule replaces a built-in default but not what a
//! built-in rule for one program sets: that fix survives launch-wide
//! settings, and a rule of the launcher for that program changes it, with
//! or without a fingerprint. Within [`ENV_VAR`], a rule for an executable
//! beats a `*` rule whatever their line order.
//!
//! - A scalar setting ends up with the value of the last rule that sets it.
//!   `default` resets it: from that rule on it is unset, as if no rule had
//!   set it, so upstream Wine decides unless a rule folded later sets it
//!   again. For most settings that is the same as one of the values, but
//!   not for `dpi_aware`: `false` makes a program unaware, `default` leaves
//!   it to the registry and the manifest.
//! - The lists accumulate in fold order.
//! - `arguments_override` replaces the arguments the rules folded before it
//!   gathered, the built-in ones included, with its own entries; the
//!   rule's own `arguments` and those of the rules folded after it still
//!   append. In a rule for a Steam app id it is ignored
//!   ([`Rule::arguments_override`]), and the rule's `arguments` still append.
//! - A `dll_overrides` entry with the order `default`, such as
//!   `mscoree,mshtml=default`, takes its modules out of the entries the
//!   rules folded before it gave them and adds none, so the registry and
//!   `WINEDLLOVERRIDES` decide for them again.
//! - The `env` entries of a built-in `*` rule without a fingerprint are
//!   defaults: one sets its variable only where the process does not have
//!   it, so a value whoever started the process exported wins, an empty
//!   one included.
//!   Every other entry, a `*` rule's of [`ENV_VAR`] included, replaces the
//!   variable, and an entry without a value removes it.
//!   [`Resolution::env_steps`] says what each entry does in a process.
//! - `wined3d` and `metal_hud` become env entries too
//!   ([`Resolution::fold_derived_env`]): `wined3d` the `WINE_D3D_CONFIG`
//!   after every rule's own entries, `metal_hud` the `MTL_HUD_` variables
//!   where the rule that set it folds, before its own entries, so an `env`
//!   entry for one of them from that rule or a later one wins.
//! - A program inherits the environment of the process that started it,
//!   the values `compatdb.so` set there included. A value a rule for one
//!   program sets for a variable with a `*` default therefore carries into
//!   the programs it starts, whose default finds the variable set. A
//!   value it removes does not: the default sets it again in the child.
//!   The settings ntdll stores for the process (`dpi_aware`,
//!   `large_address_aware`, `gcmouse`, `force_nx`, `opengl`) are its own:
//!   a child gets the value of its own rules.
//!
//! [`Resolution`] keeps where each value came from ([`Origin`],
//! [`ScalarState`]), which `compatdb.so` logs.
//!
//! ```
//! use compatdb_core::{Dxgi, Exe, Origin, Rule, Scalar, Table, VersionInfo};
//!
//! let mut table = Table::new();
//! table.push(Rule::new(Exe::Any).with_dxgi(Dxgi::Wined3d));
//!
//! // A game gets the launch-wide value over the built-in default.
//! let game = table.preview("Game.exe", &VersionInfo::default());
//! assert_eq!(game.dxgi(), Some(Dxgi::Wined3d));
//! assert_eq!(game.scalar(Scalar::Dxgi).set_by, Some(Origin::Line(1)));
//!
//! // Steam's web helper keeps the DXMT its built-in rule pins.
//! let helper = VersionInfo {
//!     company: "Valve Corporation".into(),
//!     product: "Steam Client WebHelper".into(),
//!     ..VersionInfo::default()
//! };
//! let steam = table.preview("steamwebhelper.exe", &helper);
//! assert_eq!(steam.dxgi(), Some(Dxgi::Dxmt));
//! assert!(matches!(
//!     steam.scalar(Scalar::Dxgi).set_by,
//!     Some(Origin::BuiltInFor(_))
//! ));
//!
//! // A rule of the launcher for the helper changes it.
//! table.push(Rule::new(Exe::new("steamwebhelper.exe")?).with_dxgi(Dxgi::Wined3d));
//! let steam = table.preview("steamwebhelper.exe", &helper);
//! assert_eq!(steam.dxgi(), Some(Dxgi::Wined3d));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Keys
//!
//! [`Key`] lists every field. Each variant says what it does and what a
//! process gets without it, and links to the getter of [`Rule`] with the
//! details: when it applies, which part of Wine reads it and which
//! built-in rule sets it.
//!
//! - Matchers: [`Key::Exe`], [`Key::Company`], [`Key::Product`],
//!   [`Key::OriginalFilename`].
//! - Scalar settings, each a [`Scalar`] with a [`Setting`]:
//!   [`Key::Dxgi`], [`Key::D3d9`], [`Key::Wined3d`], [`Key::OpenGl`],
//!   [`Key::DpiAware`], [`Key::LargeAddressAware`], [`Key::Gcmouse`],
//!   [`Key::ForceNx`], [`Key::MetalHud`].
//! - Lists: [`Key::DllOverrides`], [`Key::ArgumentsOverride`],
//!   [`Key::Arguments`], [`Key::DebugChannels`].
//! - Environment entries: [`Key::Env`].
//!
//! [`Key::kind`] says which of these a key is, [`Key::values`] gives the
//! spellings of a scalar setting, `default` included, [`Key::label`] its
//! name for a person, and [`Key::upstream`] what upstream Wine does
//! without it, as `compatdb.so` logs it. For a launcher's templates and
//! help, [`Key::summary`] says in one line what a key does,
//! [`Key::example`] gives a value to show and [`Key::docs_url`] links to
//! its doc; [`DocSection::url`] links to a section of this page.
//!
//! # Built-in rules
//!
//! [`builtin`] holds the rules compiled into `compatdb.so`, one function
//! each, whose doc says why the rule exists. [`builtin::defaults`] match
//! every process: a behaviour of this Wine that differs from upstream Wine
//! and that a rule can switch is off in the code, and one of these turns
//! it on, so the log of a process shows its whole configuration with the
//! source of every value, and a launcher rule that sets the same key
//! changes it. [`builtin::programs`] each name an executable and pin its
//! version resource, so none of them catches an unrelated program of the
//! same name; a test enforces this.
//!
//! # Recipes
//!
//! Each is one line of [`ENV_VAR`]; `Game.exe` stands for the game's
//! executable. [`Rule::set`] with the same keys and values builds them.
//!
//! | Goal | Rule |
//! |------|------|
//! | Re-enable Wine's Mono for every process | `exe=*;dll_overrides=mscoree=b` |
//! | Prefer a native .NET | `exe=*;dll_overrides=mscoree=n,b` |
//! | Hand Mono and Gecko back to the registry and `WINEDLLOVERRIDES` | `exe=*;dll_overrides=mscoree,mshtml=default` |
//! | Make one game DPI-unaware | `exe=Game.exe;dpi_aware=false` |
//! | Let the registry and the manifest decide DPI awareness | `exe=*;dpi_aware=default` |
//! | Give one 32-bit game 4 GB | `exe=Game.exe;large_address_aware=true` |
//! | Give one game the cursor's mouse movement | `exe=Game.exe;gcmouse=false` |
//! | Give every process wined3d for D3D10 and D3D11, and no D3D12 | `exe=*;dxgi=wined3d` |
//! | Give every process wined3d for D3D9 | `exe=*;d3d9=wined3d` |
//! | Hide D3D12 from one game, keeping DXMT for D3D10 and D3D11 | `exe=Game.exe;dll_overrides=d3d12,d3d12core=` |
//! | Change a vkd3d-proton setting for one game | `exe=Game.exe;env=VKD3D_CONFIG=<options>` |
//! | Give one game Wine's no-execute behaviour | `exe=Game.exe;force_nx=false` |
//! | Give one game FEX's full x87 precision | `exe=Game.exe;env=FEX_X87REDUCEDPRECISION=` |
//! | Give one game Apple's OpenGL | `exe=Game.exe;opengl=cgl` |
//! | Put wined3d on Vulkan for one game | `exe=Game.exe;wined3d=vulkan` |
//! | Show the Metal HUD in one game | `exe=Game.exe;metal_hud=true` |
//! | Leave one game to upstream Wine for a setting | `exe=Game.exe;<key>=default` |
//! | Replace the arguments the rules give one game | `exe=Game.exe;arguments_override=-windowed` |
//! | Give one game no arguments from any rule | `exe=Game.exe;arguments_override=` |
//! | Change what a built-in rule sets for one program | `exe=Launcher.exe;dxgi=dxmt` |
//! | Give every program that holds a Steam app id wined3d | `exe=steam:440;dxgi=wined3d` |
//! | Give one game wined3d by its app id, and not the programs around it | `exe=steam:440;product=<product>;dxgi=wined3d` |
//!
//! Most of them with `exe=*` do the same for every process, except where a
//! built-in rule for one program sets the key. [`Rule::set`] refuses
//! `arguments_override` in a `*` rule: for every process it would take away
//! the arguments the built-in rules give single programs.
//!
//! Hiding D3D12 disables both of vkd3d-proton's modules, so `d3d12.dll`
//! fails to load and a game that can use D3D11 falls back to it on DXMT; a
//! game that only has D3D12 does not start with it, nor with
//! `dxgi=wined3d`.
//!
//! # Launcher responsibilities
//!
//! - Write the value with `Table`'s `Display` and nothing else. Never
//!   write the format by hand, add a version header or send the built-in
//!   rules: `Display` leaves them out, since `compatdb.so` has them.
//! - Remove the variables of [`inherited_conflicts`] from the environment
//!   Wine starts with when the launcher inherited them, so a launch does
//!   not depend on where the launcher was started from.
//! - Build `WINEDEBUG` itself: Wine reads it before `compatdb.so` loads.
//!   Its own entries come first, then [`Table::winedebug`], joined with
//!   `,`.
//! - Ask the crate instead of naming keys: [`Key::replaces_on_merge`] when
//!   it lays one layer of its configuration over another, and
//!   [`Key::allowed_on_any`] for the settings it offers for every process.
//! - Refuse an environment entry for a variable a setting replaced
//!   ([`retired_env`]); [`Rule::set_env`] does, and [`check_env_name`]
//!   checks a name on its own. That includes
//!   `WINEDLLOVERRIDES`: an override `compatdb.so` adds replaces its entry
//!   for the same module, so it cannot undo one, while a rule's
//!   `<module>=default` can.
//! - Show or hide the Metal HUD with [`Key::MetalHud`], not with its own
//!   `MTL_HUD_` entries in `env`.
//! - Put `env` entries and `metal_hud` on the rule of the program that
//!   needs them, not on a program that starts games, such as a store
//!   client: the games it starts inherit them.
//! - Name a former built-in rule with [`builtin::former`], which says what
//!   to do instead; such a name reaches nothing any more.
//! - When passing the rules in a file, set [`FILE_ENV_VAR`] in the
//!   environment the root of the process tree starts with: a program that
//!   is already running does not get a variable set later, and neither do
//!   the programs it starts.
//! - Write such a file to a temporary file in the same directory and rename
//!   it over the path. A process that reads a file while it is being written
//!   gets a partial table, without a warning.
//! - Expect a changed table to reach only the processes that start after
//!   the change: a running process keeps the rules it read. So the
//!   processes that outlive one launch, such as `explorer.exe` and
//!   `services.exe`, get it only in the next wineserver session. With
//!   [`ENV_VAR`], so does everything else a running process starts, since
//!   it inherits the old value; with [`FILE_ENV_VAR`] it reads the new
//!   table.
//! - Expect a change of `debug_channels` in such a file to show only in the
//!   log: `WINEDEBUG` is fixed when the tree starts, so it takes a new tree
//!   for the change to take effect.
//!
//! ```
//! use compatdb_core::{Exe, FieldError, Key, Rule, Table, inherited_conflicts, retired_env};
//!
//! let mut game = Rule::new(Exe::new("Game.exe")?);
//! game.set(Key::DebugChannels, "+relay,-seh")?;
//! let mut table = Table::new();
//! table.push(game);
//! let mut winedebug = vec![String::from("-all")];
//! winedebug.extend(table.winedebug());
//! assert_eq!(winedebug.join(","), "-all,Game.exe:+relay,Game.exe:-seh");
//!
//! let mut every = Rule::new(Exe::Any);
//! let error = every.set_env("WINEDLLOVERRIDES", "mscoree=b").err();
//! assert!(matches!(
//!     error,
//!     Some(FieldError::RetiredEnv {
//!         key: Key::DllOverrides,
//!         ..
//!     })
//! ));
//! assert_eq!(retired_env("WineDebug"), Some(Key::DebugChannels));
//! assert!(inherited_conflicts().contains(&"WINEDLLOVERRIDES"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

/// The text of [`DOCS_URL`] as a literal, so `concat!` can build the links
/// of [`Key::docs_url`] and [`DocSection::url`] from it.
macro_rules! docs_url {
    () => {
        "https://docs.mach9.app/compatdb_core/"
    };
}

pub mod builtin;
mod docs;
mod error;
mod key;
mod part;
mod resolve;
mod rule;
pub mod runtime;
mod settings;
mod table;
mod value;
mod version;
mod wire;

pub use crate::{
    docs::DocSection,
    error::{Diagnostic, FieldError, ParseError, Severity},
    key::{Key, KeyKind, Scalar, ScalarValue, ValueKind},
    part::{
        Argument, DebugChannel, DllOverride, EnvEntry, EnvName, Exe, ExeName, Fingerprint,
        Specificity, SteamApp,
    },
    resolve::{
        ArgumentsDropped, DllOverrideReset, EnvFrom, EnvStep, Origin, Resolution, ScalarState,
    },
    rule::{DEFAULT_VALUE, Rule, Setting},
    settings::{
        check_env_name, inherited_conflicts, retired_env, retired_envs, retired_keys,
        retired_values,
    },
    table::{Source, Table},
    value::{D3d9, Dxgi, METAL_HUD_ENV, OpenGl, Wined3d, metal_hud_env},
    version::VersionInfo,
};

/// The environment variable `compatdb.so` reads extra rules from. Wine itself
/// never looks at it; it is only the channel from whatever started the process
/// tree to the library.
pub const ENV_VAR: &str = "WINE_COMPATDB";

/// The environment variable naming a file that holds what [`ENV_VAR`] would
/// hold, which takes its place: see [Format](index.html#format). Like
/// [`ENV_VAR`], it is only read by `compatdb.so`.
pub const FILE_ENV_VAR: &str = "WINE_COMPATDB_FILE";

/// Where this crate's rustdoc is published. [`Key::docs_url`] and
/// [`DocSection::url`] link into it.
pub const DOCS_URL: &str = docs_url!();
