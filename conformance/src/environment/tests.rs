use std::path::PathBuf;

use super::*;

fn caller(names: &[&str]) -> Vec<(OsString, OsString)> {
    names
        .iter()
        .map(|name| (OsString::from(name), OsString::from("caller")))
        .collect()
}

fn value<'e>(env: &'e [(OsString, OsString)], name: &str) -> Option<&'e OsStr> {
    let mut values = env.iter().filter(|(n, _)| n == name).map(|(_, v)| v);
    let found = values.next().map(OsString::as_os_str);
    assert!(values.next().is_none(), "{name} is set twice");
    found
}

#[test]
fn the_caller_s_wine_and_graphics_variables_are_removed() {
    let removed = [
        "WINEPREFIX",
        "WINEDEBUG",
        "WINEMSYNC",
        "WINE_COMPATDB",
        "WINE_D3D_CONFIG",
        "WINETEST_DEBUG",
        "WINETEST_REPORT_FLAKY",
        "WINETEST_PLATFORM",
        "WINETEST_COLOR",
        "FEX_HARDWARE_TSO",
        "VK_ICD_FILENAMES",
        "VKD3D_CONFIG",
        "MESA_DEBUG",
        "MTL_HUD_ENABLED",
        "METAL_DEVICE_WRAPPER_TYPE",
        "DXMT_LOG_LEVEL",
        "GST_DEBUG",
        "ZINK_DEBUG",
        "WT_ONLY",
        "WT_SKIP",
        "WT_LIST",
        "GALLIUM_DRIVER",
        "LIBGL_ALWAYS_SOFTWARE",
        "EGL_PLATFORM",
        "MVK_CONFIG_LOG_LEVEL",
        "NIR_DEBUG",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
    ];
    let kept = ["PATH", "HOME", "TMPDIR", "LANG", "VULKAN_SDK", "XWINE"];
    let names: Vec<&str> = removed.iter().chain(&kept).copied().collect();
    let env = wine_env(
        caller(&names),
        &PathBuf::from("/s/prefix"),
        &PathBuf::from("/s/work"),
        None,
    );
    for name in removed {
        assert_ne!(value(&env, name), Some(OsStr::new("caller")), "{name}");
    }
    for name in [
        "WINETEST_DEBUG",
        "WINETEST_REPORT_FLAKY",
        "WINETEST_PLATFORM",
    ] {
        assert_eq!(value(&env, name), None, "{name}");
    }
    for name in kept {
        assert_eq!(value(&env, name), Some(OsStr::new("caller")), "{name}");
    }
}

#[test]
fn the_runner_s_settings_are_set_once() {
    let env = wine_env(
        caller(&["WINEPREFIX", "PATH"]),
        &PathBuf::from("/s/prefix"),
        &PathBuf::from("/s/work"),
        None,
    );
    for (name, expected) in [
        ("WINEPREFIX", "/s/prefix"),
        ("WINEDLLOVERRIDES", "mscoree,mshtml="),
        ("WINEDEBUG", "-all"),
        ("WINETEST_NO_MT_D3D", "1"),
        ("WINETEST_TIME", "1"),
        ("VKD3D_SHADER_CACHE_PATH", "/s/work"),
    ] {
        assert_eq!(value(&env, name), Some(OsStr::new(expected)), "{name}");
    }
    assert_eq!(value(&env, "WT_ONLY"), None);
    assert_eq!(env.len(), 7);
}

#[test]
fn without_channels_the_environment_is_the_untraced_one() {
    let env = wine_env(
        caller(&["WINEDEBUG", "PATH", "HOME"]),
        &PathBuf::from("/s/prefix"),
        &PathBuf::from("/s/work"),
        None,
    );
    let expected: Vec<(OsString, OsString)> = [
        ("PATH", "caller"),
        ("HOME", "caller"),
        ("WINEPREFIX", "/s/prefix"),
        ("WINEDLLOVERRIDES", "mscoree,mshtml="),
        ("WINEDEBUG", "-all"),
        ("WINETEST_NO_MT_D3D", "1"),
        ("WINETEST_TIME", "1"),
        ("VKD3D_SHADER_CACHE_PATH", "/s/work"),
    ]
    .iter()
    .map(|(name, value)| (OsString::from(name), OsString::from(value)))
    .collect();
    assert_eq!(env, expected);
}

#[test]
fn channels_replace_only_the_value_of_winedebug() {
    let caller_env = caller(&["WINEDEBUG", "PATH"]);
    let prefix = PathBuf::from("/s/prefix");
    let work = PathBuf::from("/s/work");
    let untraced = wine_env(caller_env.clone(), &prefix, &work, None);
    let traced = wine_env(caller_env, &prefix, &work, Some("+win,+event"));
    assert_eq!(value(&traced, "WINEDEBUG"), Some(OsStr::new("+win,+event")));
    let rest = |env: &[(OsString, OsString)]| -> Vec<(OsString, OsString)> {
        env.iter()
            .filter(|(name, _)| name != "WINEDEBUG")
            .cloned()
            .collect()
    };
    assert_eq!(rest(&traced), rest(&untraced));
    assert_eq!(traced.len(), untraced.len());
}

#[test]
fn channels_are_one_word() {
    for channels in [
        "+win",
        "+win,+event,+macdrv",
        "warn+all,-heap",
        "+timestamp,+pid,+win",
    ] {
        assert_eq!(check_winedebug(channels), Ok(()), "{channels}");
    }
    assert_eq!(
        check_winedebug(""),
        Err(String::from("--winedebug names no channel"))
    );
    for channels in ["+win +event", "+win\n+event", "+win\t", "+win\u{7}"] {
        let error = check_winedebug(channels).unwrap_err();
        assert!(
            error.ends_with("has white space or a control character"),
            "{error}"
        );
    }
}

#[test]
fn deqp_processes_get_only_the_named_driver() {
    let env = deqp_env(
        caller(&[
            "VK_ICD_FILENAMES",
            "VK_ADD_DRIVER_FILES",
            "VK_DRIVER_FILES",
            "VK_INSTANCE_LAYERS",
            "MESA_KK_DEBUG",
            "DYLD_LIBRARY_PATH",
            "PATH",
        ]),
        &PathBuf::from("/m/share/vulkan/icd.d"),
        &PathBuf::from("/s/work"),
        None,
    );
    for (name, expected) in [
        ("VK_DRIVER_FILES", "/m/share/vulkan/icd.d"),
        ("VK_LOADER_LAYERS_DISABLE", "~all~"),
        ("MESA_SHADER_CACHE_DIR", "/s/work/mesa-shader-cache"),
        ("PATH", "caller"),
    ] {
        assert_eq!(value(&env, name), Some(OsStr::new(expected)), "{name}");
    }
    for name in [
        "VK_ICD_FILENAMES",
        "VK_ADD_DRIVER_FILES",
        "VK_INSTANCE_LAYERS",
        "MESA_KK_DEBUG",
        "DYLD_LIBRARY_PATH",
    ] {
        assert_eq!(value(&env, name), None, "{name}");
    }
    assert_eq!(env.len(), 4);
}

#[test]
fn a_group_s_kk_debug_replaces_the_caller_s() {
    let env = deqp_env(
        caller(&["MESA_KK_DEBUG", "MESA_KK_EXPERIMENTAL", "PATH"]),
        &PathBuf::from("/m/share/vulkan/icd.d"),
        &PathBuf::from("/s/work"),
        Some("force_icb,msl"),
    );
    assert_eq!(
        value(&env, "MESA_KK_DEBUG"),
        Some(OsStr::new("force_icb,msl"))
    );
    assert_eq!(value(&env, "MESA_KK_EXPERIMENTAL"), None);
    assert_eq!(value(&env, "PATH"), Some(OsStr::new("caller")));
    assert_eq!(env.len(), 5);
}
