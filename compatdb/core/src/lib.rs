//! The table of per-process rules behind `compatdb.so`.
//!
//! A [`Rule`] says which processes it matches and which settings it applies to
//! them: the Direct3D implementations to load (one for D3D9, one for the DXGI
//! family) and the renderer of wined3d, which OpenGL the Mac driver gives,
//! DLL load-order overrides, the DPI awareness, large address awareness,
//! whether the Mac driver reads raw mouse input from `GameController`,
//! whether no-execute stays permanently on, command-line switches and
//! environment entries. A rule has no name: it is its matchers and its
//! values.
//!
//! Shared by `compatdb.so`, the unix library wine's ntdll loads into every
//! process (it takes the built-in rules of [`builtin`], parses [`ENV_VAR`],
//! appends the result to them and applies whatever matches the process), and
//! by any launcher that wants to feed it extra rules through that variable.
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
mod process;
mod resolve;
mod rule;
mod table;
mod value;
mod version;
mod wire;

pub use crate::{
    error::FieldError,
    key::{Key, KeyKind, Scalar, ScalarValue, ValueKind},
    process::{
        Arch, IMAGE_FILE_MACHINE_ARM64, append_cmdline, append_cmdline_each, basename_of,
        env_value, merge_env,
    },
    resolve::{ArgumentsDropped, DllOverrideReset, EnvFrom, EnvStep, Origin, Resolution},
    rule::{ANY_EXE, DEFAULT_VALUE, Rule, Source},
    table::Table,
    value::{D3d9, Dxgi, OpenGl, Wined3d},
    version::VersionInfo,
};

/// The environment variable `compatdb.so` reads extra rules from. Wine itself
/// never looks at it; it is only the channel from whatever started the process
/// tree to the library.
pub const ENV_VAR: &str = "WINE_COMPATDB";
