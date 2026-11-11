use super::*;

/// The source of a launcher file.
fn launcher(file: &str) -> String {
    let path = format!("{}/../launcher/app/src/{file}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The value of `const <name>: u32 = <value>;` in a source text.
fn const_u32(text: &str, name: &str) -> u32 {
    let marker = format!("const {name}: u32 = ");
    text.lines()
        .find_map(|line| line.split_once(&marker).map(|(_, rest)| rest))
        .and_then(|rest| rest.strip_suffix(';'))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("no {name}"))
}

/// The data of the setting `name`.
fn data(name: &str) -> &'static str {
    let found = SETTINGS.iter().find(|(_, value, _, _)| *value == name);
    found.unwrap_or_else(|| panic!("no {name}")).3
}

/// The display values are what the launcher's `auto` gives on a Retina
/// display, under the keys and names the launcher writes.
#[test]
fn the_display_values_are_the_launcher_s_on_a_retina_display() {
    let dpi = launcher("dpi.rs");
    let retina_dpi = const_u32(&dpi, "UNSCALED") * const_u32(&dpi, "WINEMAC_RETINA_SCALE");
    assert_eq!(data("LogPixels"), retina_dpi.to_string());
    assert_eq!(data("RetinaMode"), "Y");
    assert_eq!(data("EmulateModeset"), "Y");
    assert_eq!(data("ShowCrashDialog"), "0");
    let registry = launcher("registry.rs");
    for (key, name, kind, _) in SETTINGS {
        let user_reg_key = key.strip_prefix(r"HKCU\").unwrap().replace('\\', r"\\");
        assert!(
            registry.contains(&format!("key: r\"{user_reg_key}\"")),
            "{key}"
        );
        assert!(registry.contains(&format!("name: \"{name}\"")), "{name}");
        assert!(registry.contains(&format!("kind: \"{kind}\"")), "{kind}");
    }
}
