//! What the output of the probe says about the Vulkan stack it ran on.
//!
//! The probe runs one case with `VK_LOADER_DEBUG=driver`, so the loader
//! prints each manifest it found and, when the case creates a device, the
//! driver library it uses for it:
//!
//! ```text
//! [Vulkan Loader] DRIVER:         Found ICD manifest file /m/share/vulkan/icd.d/kosmickrisp_mesa_icd.aarch64.json, version 1.0.1
//! [Vulkan Loader] DRIVER:                Using "Apple M5 Max" with driver: "/m/lib/libvulkan_kosmickrisp.dylib"
//! ```
//!
//! The case's test log has the device's properties, `driverVersion` among
//! them.

/// The manifest files the loader says it found, each once, in output
/// order.
#[must_use]
pub fn manifests(output: &str) -> Vec<String> {
    const FOUND: &str = "Found ICD manifest file ";
    let mut found: Vec<String> = Vec::new();
    for line in output.lines() {
        let Some(at) = line.find(FOUND) else {
            continue;
        };
        let rest = &line[at + FOUND.len()..];
        let path = rest
            .rsplit_once(", version ")
            .map_or(rest, |(path, _)| path)
            .trim();
        if !found.iter().any(|known| known == path) {
            found.push(path.to_owned());
        }
    }
    found
}

/// The device and driver library of each device the loader set up, in
/// output order.
#[must_use]
pub fn drivers(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let rest = &line[line.find("Using \"")? + 7..];
            let (device, rest) = rest.split_once("\" with driver: \"")?;
            let driver = rest.strip_suffix('"')?;
            Some((device.to_owned(), driver.to_owned()))
        })
        .collect()
}

/// The `driverVersion` of the test log's device properties as
/// `<major>.<minor>.<patch>`, decoded as `VK_MAKE_VERSION` packs it, which
/// Mesa uses.
#[must_use]
pub fn driver_version(qpa: &str) -> Option<String> {
    qpa.lines().find_map(|line| {
        let value: u32 = line
            .trim()
            .strip_prefix("driverVersion = ")?
            .trim()
            .parse()
            .ok()?;
        Some(format!(
            "{}.{}.{}",
            value >> 22,
            (value >> 12) & 0x3ff,
            value & 0xfff
        ))
    })
}

#[cfg(test)]
mod tests;
