//! The table of per-process rules behind `compatdb.so`.
//!
//! A [`Rule`] says which processes it matches and which settings it applies to
//! them: the Direct3D implementations to load (one for D3D9, one for the DXGI
//! family) and the renderer of wined3d, which OpenGL the Mac driver gives,
//! DLL load-order overrides, the DPI awareness, large address awareness,
//! whether the Mac driver reads raw mouse input from `GameController`,
//! whether no-execute stays permanently on, whether the Metal performance HUD
//! shows, command-line switches, `WINEDEBUG` channels and environment
//! entries. A rule has no name: it is its matchers and its values.
//!
//! Shared by `compatdb.so`, the unix library wine's ntdll loads into every
//! process (it takes the built-in rules of [`builtin`], parses [`ENV_VAR`],
//! appends the result to them and applies whatever matches the process), and
//! by any launcher that wants to feed it extra rules through that variable.
//!
//! A launcher fills a rule from its own settings without knowing any key:
//! it reads the name of each with [`Key::from_setting`], branches on
//! [`Key::kind`] and hands the value as text to [`Rule::set`],
//! [`Rule::set_list`] or [`Rule::set_env`]. Every error is a [`FieldError`]
//! to print as it is, and its [`FieldError::severity`] says whether to warn
//! and go on or to refuse the settings. [`builtin::former`] says what to do
//! instead of naming a former built-in rule. [`Table::preview`] shows what a
//! program would get. The [`runtime`] module holds what only `compatdb.so`
//! needs.
//!
//! Everything here is pure: it works on `&str`, `&[u16]` (UTF-16 as wine hands
//! it over) and owned values, with no unsafe and no I/O, so the library's
//! decision logic is all unit-tested on the host and the `.so` itself only has
//! to move bytes in and out of the process.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub mod builtin;
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
    error::{Diagnostic, FieldError, ParseError, Severity},
    key::{Key, KeyKind, Scalar, ScalarValue, ValueKind},
    part::{
        Argument, DebugChannel, DllOverride, EnvEntry, EnvName, Exe, ExeName, Fingerprint,
        Specificity,
    },
    resolve::{
        ArgumentsDropped, DllOverrideReset, EnvFrom, EnvStep, Origin, Resolution, ScalarState,
    },
    rule::{DEFAULT_VALUE, Rule, Setting},
    settings::{inherited_conflicts, retired_env},
    table::{Source, Table},
    value::{D3d9, Dxgi, METAL_HUD_ENV, OpenGl, Wined3d, metal_hud_env},
    version::VersionInfo,
};

/// The environment variable `compatdb.so` reads extra rules from. Wine itself
/// never looks at it; it is only the channel from whatever started the process
/// tree to the library.
pub const ENV_VAR: &str = "WINE_COMPATDB";
