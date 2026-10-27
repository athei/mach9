//! The `retina_mode` setting: whether winemac draws at backing resolution.
//!
//! `auto`, the default, turns it on when the main display is high-density: its
//! current mode has more pixels than points, which is the width against pixel
//! width comparison winemac itself makes on a display mode. On a 1x display,
//! such as most external monitors, retina mode would only make winemac render
//! twice the pixels for the window server to scale back down.

use std::ffi::c_void;

use log::warn;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RetinaMode {
    /// On when the main display is high-density.
    #[default]
    Auto,
    On,
    Off,
}

/// The forms the setting takes, for error messages.
pub const VALUES: &str = "`auto`, `true` or `false`";

impl RetinaMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "true" => Some(Self::On),
            "false" => Some(Self::Off),
            _ => None,
        }
    }

    /// Whether retina mode is on for this launch. `auto` asks the display.
    pub fn resolve(self) -> bool {
        match self {
            Self::Auto => main_display_is_hidpi().unwrap_or_else(|| {
                warn!("cannot read the main display's mode; assuming a HiDPI display");
                true
            }),
            Self::On => true,
            Self::Off => false,
        }
    }
}

/// Whether the main display's current mode has more pixels than points.
/// `None` when there is no mode to read, as in a session without a display.
fn main_display_is_hidpi() -> Option<bool> {
    // SAFETY: plain CoreGraphics queries. The mode is checked for null before
    // use and released once, after the last read.
    unsafe {
        let mode = CGDisplayCopyDisplayMode(CGMainDisplayID());
        if mode.is_null() {
            return None;
        }
        let points = CGDisplayModeGetWidth(mode);
        let pixels = CGDisplayModeGetPixelWidth(mode);
        CGDisplayModeRelease(mode);
        Some(pixels > points)
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auto_and_the_booleans_only() {
        assert_eq!(RetinaMode::parse("auto"), Some(RetinaMode::Auto));
        assert_eq!(RetinaMode::parse("true"), Some(RetinaMode::On));
        assert_eq!(RetinaMode::parse("false"), Some(RetinaMode::Off));
        for text in ["", "Auto", "on", "off", "0", "1", "yes"] {
            assert_eq!(RetinaMode::parse(text), None, "{text:?}");
        }
        assert!(RetinaMode::On.resolve());
        assert!(!RetinaMode::Off.resolve());
    }
}
