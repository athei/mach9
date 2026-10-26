use super::*;
use crate::env_value;

/// Every arch a program can have, which every implementation serves.
const ARCHES: [Arch; 3] = [Arch::X86_64, Arch::I386, Arch::Aarch64];

#[test]
fn without_a_rule_every_arch_gets_wined3d_and_otherwise_what_it_asks_for() {
    // No opinion means upstream Wine's implementation of both families;
    // the built-in rules of compatdb.so ask for DXMT and mtld3d.
    for arch in ARCHES {
        assert_eq!(Dxgi::default_for(arch), Dxgi::Wined3d, "{arch:?}");
        assert_eq!(Dxgi::effective(None, arch), Dxgi::Wined3d, "{arch:?}");
        for dxgi in [Dxgi::Dxmt, Dxgi::Wined3d] {
            assert_eq!(Dxgi::effective(Some(dxgi), arch), dxgi);
        }
        assert_eq!(D3d9::default_for(arch), D3d9::Wined3d, "{arch:?}");
        assert_eq!(D3d9::effective(None, arch), D3d9::Wined3d, "{arch:?}");
        for d3d9 in [D3d9::Mtld3d, D3d9::Wined3d] {
            assert_eq!(D3d9::effective(Some(d3d9), arch), d3d9);
        }
    }
    assert_eq!(Dxgi::Wined3d.as_str(), "wined3d");
    assert_eq!(Dxgi::parse("DXMT"), Some(Dxgi::Dxmt));
    assert_eq!(Dxgi::parse("default"), None);
}

#[test]
fn d3d9_values_parse_and_print() {
    assert_eq!(D3d9::parse("wined3d"), Some(D3d9::Wined3d));
    assert_eq!(D3d9::parse("MTLD3D"), Some(D3d9::Mtld3d));
    assert_eq!(D3d9::parse("gptk"), None);
    assert_eq!(D3d9::Mtld3d.as_str(), "mtld3d");
}

#[test]
fn the_renderer_of_a_config_and_a_variable_of_a_block_read_as_wined3d_reads_them() {
    assert_eq!(
        Wined3d::renderer_in("csmt=0,renderer=vulkan"),
        Some("vulkan")
    );
    assert_eq!(
        Wined3d::renderer_in("renderer=gl;renderer=vulkan"),
        Some("gl")
    );
    assert_eq!(Wined3d::renderer_in("csmt=0,max_renderer=x"), None);
    assert_eq!(Wined3d::renderer_in(""), None);
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
