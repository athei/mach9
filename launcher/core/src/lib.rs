//! What the launcher, the Dock tile plug-in and the opener share: where a
//! bundle's files live, how the saved games are listed, selected and
//! created, and how a command-line word is quoted. No `AppKit` and no config
//! parsing, so the plug-in and the opener stay small.

pub mod bundle;
pub mod docktile;
pub mod fsutil;
pub mod games;
pub mod words;
