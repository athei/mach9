use std::{fs, path::PathBuf};

use super::*;
use crate::DOCS_URL;

/// The id rustdoc gives a heading: lowercase, a `-` for each space, and
/// only letters, digits, `-` and `_` kept.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// The headings of the crate's front page, in order, outside its code
/// blocks, whose hidden lines also start with `# `.
fn headings() -> Vec<String> {
    let mut fenced = false;
    let mut headings = Vec::new();
    for line in include_str!("../lib.rs").lines() {
        let Some(line) = line.strip_prefix("//!") else {
            continue;
        };
        let line = line.trim_start();
        if line.starts_with("```") {
            fenced = !fenced;
        } else if !fenced
            && let Some(heading) = line.strip_prefix("## ").or_else(|| line.strip_prefix("# "))
        {
            headings.push(heading.to_owned());
        }
    }
    headings
}

/// The ids of the generated rustdoc page `page`, or `None` when the docs
/// have not been built.
fn rustdoc_ids(page: &str) -> Option<String> {
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        PathBuf::from,
    );
    fs::read_to_string(target.join("doc/compatdb_core").join(page)).ok()
}

#[test]
fn every_section_links_to_a_heading_of_the_front_page() {
    let headings = headings();
    assert_eq!(headings.len(), DocSection::ALL.len(), "{headings:?}");
    let page = rustdoc_ids("index.html");
    for (&section, heading) in DocSection::ALL.iter().zip(&headings) {
        let anchor = slug(heading);
        assert_eq!(section.url(), format!("{DOCS_URL}#{anchor}"), "{section:?}");
        if let Some(page) = &page {
            assert!(page.contains(&format!("id=\"{anchor}\"")), "{anchor}");
        }
    }
    assert_eq!(
        DocSection::LauncherResponsibilities.url(),
        "https://docs.mach9.app/compatdb_core/#launcher-responsibilities"
    );
}

#[test]
fn every_key_links_to_its_variant() {
    let page = rustdoc_ids("enum.Key.html");
    for &key in Key::ALL {
        let anchor = format!("variant.{key:?}");
        assert_eq!(
            key.docs_url(),
            format!("{DOCS_URL}enum.Key.html#{anchor}"),
            "{key}"
        );
        if let Some(page) = &page {
            assert!(page.contains(&format!("id=\"{anchor}\"")), "{anchor}");
        }
    }
    assert_eq!(
        Key::MetalHud.docs_url(),
        "https://docs.mach9.app/compatdb_core/enum.Key.html#variant.MetalHud"
    );
}
