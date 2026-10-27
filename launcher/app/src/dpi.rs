//! The `dpi` setting: the Windows system DPI the prefix runs at.
//!
//! With retina mode on, winemac gives Windows the display in backing pixels,
//! a fixed two per macOS point whatever the display's "Looks like" setting is
//! (`cgsize_win_from_mac` in `winemac.drv/macdrv_cocoa.h`). At Windows' default
//! 96 DPI every window, font and dialog then comes out half the size of the
//! macOS UI around it. `auto` scales the DPI by the same factor, so one
//! Windows pixel at 96 DPI stays one macOS point, and the macOS scaling
//! setting carries over because Windows pixels follow points.

/// Windows' own default, which is what no scaling means.
pub const UNSCALED: u32 = 96;

/// Windows pixels per macOS point with winemac's retina mode on.
const WINEMAC_RETINA_SCALE: u32 = 2;

/// The range accepted for a pinned value.
const RANGE: std::ops::RangeInclusive<u32> = 48..=480;

/// What the forms the setting takes are, for error messages.
pub const VALUES: &str = "`auto`, `off` or `0` (no scaling, 96), or a DPI from 48 to 480";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dpi {
    /// Follow retina mode.
    #[default]
    Auto,
    /// This value, whatever the retina mode.
    Fixed(u32),
}

impl Dpi {
    /// `auto`, `off`, or a number; 0 means the same as `off`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "off" => Some(Self::Fixed(UNSCALED)),
            _ => Self::from_number(text.parse().ok()?),
        }
    }

    fn from_number(number: i64) -> Option<Self> {
        if number == 0 {
            return Some(Self::Fixed(UNSCALED));
        }
        let dpi = u32::try_from(number).ok()?;
        RANGE.contains(&dpi).then_some(Self::Fixed(dpi))
    }

    /// The DPI to run at with the given retina mode.
    pub const fn resolve(self, retina: bool) -> u32 {
        match self {
            Self::Auto if retina => UNSCALED * WINEMAC_RETINA_SCALE,
            Self::Auto => UNSCALED,
            Self::Fixed(dpi) => dpi,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_follows_retina_mode() {
        assert_eq!(Dpi::Auto.resolve(true), 192);
        assert_eq!(Dpi::Auto.resolve(false), 96);
        assert_eq!(Dpi::Fixed(144).resolve(true), 144);
        assert_eq!(Dpi::Fixed(144).resolve(false), 144);
    }

    #[test]
    fn off_and_zero_mean_no_scaling() {
        assert_eq!(Dpi::parse("auto"), Some(Dpi::Auto));
        assert_eq!(Dpi::parse("off"), Some(Dpi::Fixed(96)));
        assert_eq!(Dpi::parse("0"), Some(Dpi::Fixed(96)));
        assert_eq!(Dpi::parse("144"), Some(Dpi::Fixed(144)));
    }

    #[test]
    fn rejects_values_outside_the_range_and_other_words() {
        for text in ["", "Auto", "on", "true", "-96", "47", "481", "1.5", "96dpi"] {
            assert_eq!(Dpi::parse(text), None, "{text:?}");
        }
        assert_eq!(Dpi::parse(&i64::MAX.to_string()), None);
        assert_eq!(Dpi::parse("48"), Some(Dpi::Fixed(48)));
        assert_eq!(Dpi::parse("480"), Some(Dpi::Fixed(480)));
    }
}
