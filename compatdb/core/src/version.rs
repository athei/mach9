//! The version resource of an image, which a rule's fingerprint matches on.

#[cfg(doc)]
use crate::runtime::version_info;

/// The parts of a PE version resource the database can match on. Vendor-set at
/// link time, so stable across install location and patches.
///
/// A field is an empty string when the image carries no version resource or
/// the resource leaves that field empty. `compatdb.so` reads it from the
/// image with [`version_info`]; a launcher that wants to see what a program
/// would get fills it in itself.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct VersionInfo {
    /// The `CompanyName` string.
    pub company: String,
    /// The `ProductName` string.
    pub product: String,
    /// The `OriginalFilename` string.
    pub original_filename: String,
}
