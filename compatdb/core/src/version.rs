//! The version resource of an image, which a rule's fingerprint matches on,
//! and the reader of a `VS_VERSIONINFO` blob.

use crate::process::to_lower_u16;

/// The parts of a PE version resource the database can match on. Vendor-set at
/// link time, so stable across install location and patches.
///
/// A field is an empty string when the image carries no version resource or
/// the resource leaves that field empty.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct VersionInfo {
    /// The `CompanyName` string.
    pub company: String,
    /// The `ProductName` string.
    pub product: String,
    /// The `OriginalFilename` string.
    pub original_filename: String,
}

impl VersionInfo {
    /// Extract the standard fields from a `VS_VERSIONINFO` blob (the bytes of an
    /// `RT_VERSION` resource, as UTF-16 units). The blob is walked as the tree
    /// it is: the root, its `StringFileInfo` children, their string tables and
    /// the `String` entries in them. A field takes the value of the first entry
    /// with its key, in table order; an entry whose value is empty, or a key no
    /// table has, leaves the field empty, so a rule that pins it cannot match.
    #[must_use]
    pub fn from_resource(blob: &[u16]) -> Self {
        Self {
            company: string_value(blob, "CompanyName"),
            product: string_value(blob, "ProductName"),
            original_filename: string_value(blob, "OriginalFilename"),
        }
    }
}

/// One node of a `VS_VERSIONINFO` tree. Every node, the root, the containers
/// and the `String` entries alike, is `wLength` (bytes, children included),
/// `wValueLength`, `wType`, a NUL-terminated UTF-16 key, padding to a 32-bit
/// boundary, the value, padding again, and then the children.
struct VersionNode<'a> {
    key: &'a [u16],
    /// From the start of the value to the end of the node.
    value: &'a [u16],
    /// The children, as the units that hold them.
    children: &'a [u16],
}

impl<'a> VersionNode<'a> {
    /// Parse the node at the start of `units` and return it with the units
    /// after it, where its next sibling starts. `units` has to start on a
    /// 32-bit boundary, which every node does. A node whose length is too
    /// short for its header or whose key is not NUL-terminated ends the walk;
    /// one that claims more than `units` holds is cut off at the end.
    fn parse(units: &'a [u16]) -> Option<(Self, &'a [u16])> {
        let length = usize::from(*units.first()?).div_ceil(2);
        let value_length = usize::from(*units.get(1)?);
        // Any nonzero wType is text, as in Wine's VerQueryValue.
        let text = *units.get(2)? != 0;
        if length < 3 {
            return None;
        }
        let node = units.get(..length).unwrap_or(units);
        let key_text = node.get(3..)?;
        let key_end = key_text.iter().position(|&u| u == 0)?;
        let key = key_text.get(..key_end)?;
        // The key starts 3 units in, and its NUL is one more.
        let value_at = key_end.checked_add(4)?.checked_next_multiple_of(2)?;
        let value = node.get(value_at..).unwrap_or_default();
        // wValueLength counts characters for a text value and bytes for a
        // binary one; the children start at the next 32-bit boundary after it.
        let value_units = if text {
            value_length
        } else {
            value_length.div_ceil(2)
        };
        let children_at = value_at
            .checked_add(value_units)?
            .checked_next_multiple_of(2)?;
        let children = node.get(children_at..).unwrap_or_default();
        let next = length.checked_next_multiple_of(2)?;
        let rest = units.get(next..).unwrap_or_default();
        Some((
            Self {
                key,
                value,
                children,
            },
            rest,
        ))
    }

    /// The nodes in a run of sibling units, in order.
    fn siblings(mut units: &'a [u16]) -> impl Iterator<Item = Self> {
        std::iter::from_fn(move || {
            let (node, rest) = Self::parse(units)?;
            units = rest;
            Some(node)
        })
    }

    /// Whether the key is `want`, compared ASCII-case-insensitively as
    /// `VerQueryValue` does.
    fn key_is(&self, want: &str) -> bool {
        self.key
            .iter()
            .map(|&u| to_lower_u16(u))
            .eq(want.encode_utf16().map(to_lower_u16))
    }
}

/// The value of the first `String` entry with this key in any string table of
/// the blob, empty when there is none or its value is empty.
///
/// The value is read up to its NUL within the entry rather than by
/// `wValueLength`, which resource compilers do not all fill in the same way.
fn string_value(blob: &[u16], key: &str) -> String {
    let Some((root, _)) = VersionNode::parse(blob) else {
        return String::new();
    };
    if !root.key_is("VS_VERSION_INFO") {
        return String::new();
    }
    VersionNode::siblings(root.children)
        .filter(|info| info.key_is("StringFileInfo"))
        .flat_map(|info| VersionNode::siblings(info.children))
        .flat_map(|table| VersionNode::siblings(table.children))
        .find(|entry| entry.key_is(key))
        .and_then(|entry| entry.value.split(|&u| u == 0).next())
        .map(String::from_utf16_lossy)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
