use super::*;

const OUTPUT: &str = "[Vulkan Loader] DRIVER:         Searching for driver manifest files\n[Vulkan \
                      Loader] DRIVER:               /m/share/vulkan/icd.d/kk.json\n[Vulkan \
                      Loader] DRIVER:         Found ICD manifest file \
                      /m/share/vulkan/icd.d/kk.json, version 1.0.1\n[Vulkan Loader] DRIVER:         \
                      Found ICD manifest file /m/share/vulkan/icd.d/kk.json, version 1.0.1\n[Vulkan \
                      Loader] DRIVER:                Using \"Apple M5 Max\" with driver: \
                      \"/m/lib/libvulkan_kosmickrisp.dylib\"\n";

#[test]
fn the_manifests_are_listed_once() {
    assert_eq!(manifests(OUTPUT), ["/m/share/vulkan/icd.d/kk.json"]);
    assert_eq!(manifests("nothing\n").len(), 0);
}

#[test]
fn each_device_names_its_driver() {
    assert_eq!(
        drivers(OUTPUT),
        [(
            String::from("Apple M5 Max"),
            String::from("/m/lib/libvulkan_kosmickrisp.dylib")
        )]
    );
}

#[test]
fn the_driver_version_is_decoded() {
    let qpa = "#sessionInfo deviceName Apple M5 Max\n\tapiVersion = 4211055\n\tdriverVersion = \
               109457507\n";
    assert_eq!(driver_version(qpa).as_deref(), Some("26.99.99"));
    assert_eq!(driver_version("\tdriverVersion = x\n"), None);
}
