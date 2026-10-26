use super::*;

/// Every arch a program can have, which every implementation serves.
const ARCHES: [Arch; 3] = [Arch::X86_64, Arch::I386, Arch::Aarch64];

#[test]
fn without_a_rule_every_arch_gets_wined3d_and_otherwise_what_it_asks_for() {
    // No opinion means upstream Wine's implementation of both families;
    // the built-in rules of compatdb.so ask for DXMT and mtld3d.
    for arch in ARCHES {
        assert_eq!(default_dxgi(arch), Dxgi::Wined3d, "{arch:?}");
        assert_eq!(effective_dxgi(None, arch), Dxgi::Wined3d, "{arch:?}");
        for dxgi in [Dxgi::Dxmt, Dxgi::Wined3d] {
            assert_eq!(effective_dxgi(Some(dxgi), arch), dxgi);
        }
        assert_eq!(default_d3d9(arch), D3d9::Wined3d, "{arch:?}");
        assert_eq!(effective_d3d9(None, arch), D3d9::Wined3d, "{arch:?}");
        for d3d9 in [D3d9::Mtld3d, D3d9::Wined3d] {
            assert_eq!(effective_d3d9(Some(d3d9), arch), d3d9);
        }
    }
}

#[test]
fn the_opengl_hook_takes_one_for_egl() {
    assert_eq!(opengl_hook_value(OpenGl::Egl), 1);
    assert_eq!(opengl_hook_value(OpenGl::Cgl), 0);
}

#[test]
fn the_renderer_of_a_config_and_a_variable_of_a_block_read_as_wined3d_reads_them() {
    assert_eq!(
        wined3d_renderer_in("csmt=0,renderer=vulkan"),
        Some("vulkan")
    );
    assert_eq!(
        wined3d_renderer_in("renderer=gl;renderer=vulkan"),
        Some("gl")
    );
    assert_eq!(wined3d_renderer_in("csmt=0,max_renderer=x"), None);
    assert_eq!(wined3d_renderer_in(""), None);
    let block: Vec<u16> = "=C:=C:\\\0wine_d3d_config=renderer=vulkan\0WINE_D3D_CONFIG=x\0\0"
        .encode_utf16()
        .collect();
    assert_eq!(
        env_value(&block, "WINE_D3D_CONFIG").as_deref(),
        Some("renderer=vulkan")
    );
    assert_eq!(env_value(&block, "OTHER"), None);
    let empty: Vec<u16> = "A=\0\0".encode_utf16().collect();
    assert_eq!(env_value(&empty, "a").as_deref(), Some(""));
}

#[test]
fn the_wined3d_config_puts_the_renderer_first_and_keeps_the_other_options() {
    assert_eq!(wined3d_config(Wined3d::Gl, None), "renderer=gl");
    assert_eq!(
        wined3d_config(
            Wined3d::Vulkan,
            Some("renderer=no3d,csmt=0x1;MaxVersionGL=0x40001")
        ),
        "renderer=vulkan,csmt=0x1,MaxVersionGL=0x40001"
    );
    assert_eq!(
        wined3d_config(Wined3d::Vulkan, Some(",,")),
        "renderer=vulkan"
    );
}
