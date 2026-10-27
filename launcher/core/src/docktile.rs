//! How the Dock tile plug-in and its app find each other.
//!
//! Every pinned app's tile plug-in loads into one Dock plug-in host with a
//! single Objective-C class namespace, so the class a plug-in's
//! `NSPrincipalClass` names has to be this app's alone. It is named after the
//! bundle id, and the plug-in, which runs in the Dock's process rather than
//! the app's, goes the other way: from its class name to the plug-in bundle
//! that names it, and from there to the app and its support directory.

use std::{io, path::Path};

use crate::bundle::plist_string;

/// Prefix of every principal class name the plug-in answers to. Other
/// vendors' Dock tile plug-ins share the host, and their classes are theirs.
pub const CLASS_PREFIX: &str = "GameLauncherDockTile_";

/// Objective-C class the Dock tile plug-in registers for a bundle.
///
/// The step that installs the plug-in into the app writes the same name into
/// the plug-in's `Info.plist` as `NSPrincipalClass`; the two have to match.
#[must_use]
pub fn class_name(bundle_id: &str) -> String {
    let suffix: String = bundle_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{CLASS_PREFIX}{suffix}")
}

/// `NSPrincipalClass` of the plug-in bundle at `plugin`.
///
/// # Errors
///
/// As [`plist_string`] for the plug-in's `Contents/Info.plist`.
pub fn principal_class(plugin: &Path) -> io::Result<String> {
    plist_string(&plugin.join("Contents/Info.plist"), "NSPrincipalClass")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_name_keeps_only_alphanumerics() {
        assert_eq!(
            class_name("com.example.My-App"),
            "GameLauncherDockTile_com_example_My_App"
        );
        assert_eq!(class_name("app.mach9"), "GameLauncherDockTile_app_mach9");
        assert!(class_name("app.mach9").starts_with(CLASS_PREFIX));
    }
}
