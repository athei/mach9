use super::*;

#[test]
fn dxgi_values_parse_and_print() {
    assert_eq!(Dxgi::Wined3d.as_str(), "wined3d");
    assert_eq!("DXMT".parse(), Ok(Dxgi::Dxmt));
    // `default` resets a setting; it is no implementation.
    assert_eq!(
        "default".parse::<Dxgi>(),
        Err(FieldError::BadValue {
            key: Key::Dxgi,
            value: "default".into(),
            expected: &["dxmt", "wined3d"],
        })
    );
}

#[test]
fn d3d9_values_parse_and_print() {
    assert_eq!("wined3d".parse(), Ok(D3d9::Wined3d));
    assert_eq!("MTLD3D".parse(), Ok(D3d9::Mtld3d));
    assert!("gptk".parse::<D3d9>().is_err());
    assert_eq!(D3d9::Mtld3d.as_str(), "mtld3d");
}

#[test]
fn every_value_reads_back_from_its_lowercase_spelling_in_any_case() {
    fn check<T>(all: &[T])
    where
        T: Copy + PartialEq + std::fmt::Debug + std::fmt::Display + FromStr<Err = FieldError>,
    {
        for &value in all {
            let text = value.to_string();
            assert_eq!(text, text.to_ascii_lowercase());
            assert_eq!(text.parse::<T>(), Ok(value));
            assert_eq!(text.to_ascii_uppercase().parse::<T>(), Ok(value));
        }
    }
    check(Dxgi::ALL);
    check(D3d9::ALL);
    check(Wined3d::ALL);
    check(OpenGl::ALL);
}

#[test]
fn metal_hud_sets_its_three_variables_or_removes_them() {
    let shown: Vec<String> = metal_hud_env(true)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        shown,
        [
            "MTL_HUD_ENABLED=1",
            "MTL_HUD_ALIGNMENT=28",
            "MTL_HUD_ELEMENTS=device,rosetta,layersize,memory,fps,frameinterval,shaders,\
             frameintervalgraph,gputime,thermal,layerscale,refreshrate,gamemode,client",
        ]
    );
    // Off removes the same variables, so a process does not inherit the
    // HUD of the process that started it.
    let off = metal_hud_env(false);
    assert_eq!(off.len(), METAL_HUD_ENV.len());
    for (entry, (name, _)) in off.iter().zip(METAL_HUD_ENV) {
        assert_eq!(entry.name().as_str(), name);
        assert_eq!(entry.value(), None, "{name}");
    }
}
