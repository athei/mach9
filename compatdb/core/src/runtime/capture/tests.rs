use super::*;

#[test]
fn the_capture_file_is_named_after_the_executable_without_exe_and_the_pid() {
    assert_eq!(
        vulkan_capture_file("/g/captures", "Game.exe", 42),
        "/g/captures/Game-42.gfxr"
    );
    assert_eq!(
        vulkan_capture_file("/g/captures", "game.EXE", 7),
        "/g/captures/game-7.gfxr"
    );
    assert_eq!(vulkan_capture_file("/c", "tool", 1), "/c/tool-1.gfxr");
    assert_eq!(vulkan_capture_file("/c", ".exe", 1), "/c/.exe-1.gfxr");
    assert_eq!(vulkan_capture_file("/c", "ä.exe", 1), "/c/ä-1.gfxr");
}

#[test]
fn the_env_loads_the_layer_in_trimming_mode_for_this_program_alone() -> Result<(), FieldError> {
    let env: Vec<String> = vulkan_capture_env(
        "/w/share/vulkan/explicit_layer.d",
        "/g/c/G-1.gfxr",
        "G",
        |_| None,
    )?
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        env,
        [
            "VK_ADD_LAYER_PATH=/w/share/vulkan/explicit_layer.d",
            "VK_INSTANCE_LAYERS=VK_LAYER_LUNARG_gfxreconstruct",
            "GFXRECON_CAPTURE_FILE=/g/c/G-1.gfxr",
            "GFXRECON_CAPTURE_FILE_TIMESTAMP=true",
            "GFXRECON_CAPTURE_TRIGGER=win32u",
            "GFXRECON_CAPTURE_PROCESS_NAME=G",
            "GFXRECON_MEMORY_TRACKING_MODE=unassisted",
        ]
    );
    Ok(())
}

#[test]
fn the_layer_joins_the_lists_the_process_has_on_its_side_once() -> Result<(), FieldError> {
    let current = |name: &str| match name {
        "VK_ADD_LAYER_PATH" => Some(String::from("/a::/w:/b")),
        "VK_INSTANCE_LAYERS" => Some(String::from(
            "VK_LAYER_LUNARG_gfxreconstruct:VK_LAYER_KHRONOS_validation",
        )),
        _ => None,
    };
    let env = vulkan_capture_env("/w", "/f", "G", current)?;
    let values: Vec<String> = env.iter().take(2).map(ToString::to_string).collect();
    assert_eq!(
        values,
        [
            "VK_ADD_LAYER_PATH=/w:/a:/b",
            "VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation:VK_LAYER_LUNARG_gfxreconstruct",
        ]
    );
    assert_eq!(list_with(Some(""), "x", false), "x");
    assert_eq!(list_with(Some("a:b"), "x", false), "a:b:x");
    assert_eq!(list_with(Some("a:b"), "x", true), "x:a:b");
    Ok(())
}

#[test]
fn a_nul_in_a_path_is_refused() {
    assert_eq!(
        vulkan_capture_env("/w\0", "/f", "G", |_| None),
        Err(FieldError::Nul)
    );
    assert_eq!(
        vulkan_capture_env("/w", "/f", "G", |_| Some(String::from("a\0"))),
        Err(FieldError::Nul)
    );
}

#[test]
fn the_metal_env_loads_metals_capture_layer_and_arms_kosmickrisp() -> Result<(), FieldError> {
    let env: Vec<String> = metal_capture_env("/g/captures", "Game.EXE", 9)?
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        env,
        [
            "MTL_CAPTURE_ENABLED=1",
            "MESA_KK_GPU_CAPTURE=1",
            "MESA_KK_GPU_CAPTURE_DIRECTORY=/g/captures",
            "DXMT_CAPTURE_EXECUTABLE=Game",
            "DXMT_CAPTURE_FILE=/g/captures/Game-9-dxmt",
        ]
    );
    assert_eq!(metal_capture_env("/g\0", "G.exe", 1), Err(FieldError::Nul));
    Ok(())
}

#[test]
fn the_hook_value_has_a_bit_for_each_capture() {
    assert_eq!(capture_hook_value(false, false), 0);
    assert_eq!(capture_hook_value(true, false), 1);
    assert_eq!(capture_hook_value(false, true), 2);
    assert_eq!(capture_hook_value(true, true), 3);
}
