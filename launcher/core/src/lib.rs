//! What the launcher and the Dock tile plug-in share: where a bundle's files
//! live and how the saved games are listed, selected and created. No `AppKit`
//! and no config parsing, so the plug-in stays small.

pub mod bundle;
pub mod docktile;
pub mod fsutil;
pub mod games;
