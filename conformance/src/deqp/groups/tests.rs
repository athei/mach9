use super::*;

#[test]
fn glob_matches_runs_of_any_characters() {
    assert!(glob("dEQP-VK.draw.*", "dEQP-VK.draw.renderpass.simple"));
    assert!(glob("dEQP-VK.draw.*", "dEQP-VK.draw."));
    assert!(!glob("dEQP-VK.draw.*", "dEQP-VK.drawx"));
    assert!(glob(
        "dEQP-VK.query_pool.*occlusion*",
        "dEQP-VK.query_pool.occlusion_query.x"
    ));
    assert!(glob(
        "dEQP-VK.query_pool.*occlusion*",
        "dEQP-VK.query_pool.a.b_occlusion"
    ));
    assert!(!glob(
        "dEQP-VK.query_pool.*occlusion*",
        "dEQP-VK.query_pool.statistics"
    ));
    assert!(glob("dEQP-VK.a.b", "dEQP-VK.a.b"));
    assert!(!glob("dEQP-VK.a.b", "dEQP-VK.a.bc"));
    assert!(glob("*ab*b", "xabyb"));
    assert!(!glob("*ab*b", "xab"));
    assert!(glob("*", ""));
    assert!(glob("a*a", "aa"));
    assert!(!glob("a*a", "a"));
}

#[test]
fn the_committed_groups_parse_and_name_their_issues() {
    let groups = Groups::builtin().unwrap();
    let names: Vec<&str> = groups.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "draw",
            "robust-buffer-access",
            "synchronization",
            "synchronization2",
            "memory-model",
            "timestamp"
        ]
    );
    for group in &groups.groups {
        assert!(!group.issues.is_empty(), "{}", group.name);
    }
}

#[test]
fn select_takes_the_matching_cases_in_order_and_every_nth() {
    let groups = Groups::parse(
        "[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.a.*\", \"dEQP-VK.c.x\"]\nfraction = 2\n",
    )
    .unwrap();
    let mustpass = [
        "dEQP-VK.a.1",
        "dEQP-VK.b.1",
        "dEQP-VK.a.2",
        "dEQP-VK.c.x",
        "dEQP-VK.a.3",
    ];
    assert_eq!(
        groups.groups[0].select(&mustpass),
        ["dEQP-VK.a.1", "dEQP-VK.c.x"]
    );
}

#[test]
fn exclude_leaves_out_matching_cases() {
    let groups = Groups::parse(
        "[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.a.*\"]\nexclude = [\"dEQP-VK.a.*.rgen\"]\n",
    )
    .unwrap();
    let mustpass = [
        "dEQP-VK.a.x.comp",
        "dEQP-VK.a.x.rgen",
        "dEQP-VK.a.y.rgen_compute",
    ];
    assert_eq!(
        groups.groups[0].select(&mustpass),
        ["dEQP-VK.a.x.comp", "dEQP-VK.a.y.rgen_compute"]
    );
    let e =
        Groups::parse("[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.a.*\"]\nexclude = [\"rgen\"]\n")
            .err()
            .unwrap();
    assert!(e.contains("each of `exclude` is a pattern"), "{e}");
}

#[test]
fn memory_model_leaves_out_only_the_u64_atomic_rmw_fragment_cases() {
    let groups = Groups::builtin().unwrap();
    let group = groups.select(&[String::from("memory-model")]).unwrap()[0];
    let base = "dEQP-VK.memory_model.write_after_read.ext.u64.coherent.atomic_atomic";
    for (case, kept) in [
        (
            format!("{base}.atomicrmw.subgroup.payload_local.buffer.guard_local.buffer.frag"),
            false,
        ),
        (
            format!("{base}.atomicrmw.device.payload_local.image.guard_local.buffer.comp"),
            true,
        ),
        (
            format!("{base}.atomicrmw.device.payload_nonlocal.image.guard_local.buffer.frag"),
            true,
        ),
        (
            format!("{base}.atomicwrite.device.payload_local.image.guard_local.buffer.frag"),
            true,
        ),
        (
            String::from(
                "dEQP-VK.memory_model.message_passing.ext.u32.coherent.atomic_atomic.atomicrmw.\
                 device.payload_local.buffer.guard_local.buffer.frag",
            ),
            true,
        ),
        (
            String::from(
                "dEQP-VK.memory_model.message_passing.ext.u64.noncoherent.atomic_atomic.\
                 atomicrmw.queuefamily.payload_local.physbuffer.guard_local.image.frag",
            ),
            false,
        ),
    ] {
        assert_eq!(group.matches(&case), kept, "{case}");
    }
}

#[test]
fn bad_groups_are_refused() {
    for (text, error) in [
        ("[[group]]\nname = \"g\"\n", "missing `cases`"),
        ("[[group]]\nname = \"g\"\ncases = []\n", "`cases` is empty"),
        (
            "[[group]]\nname = \"g\"\ncases = [\"draw.*\"]\n",
            "starts with `dEQP-VK.`",
        ),
        (
            "[[group]]\nname = \"g h\"\ncases = [\"dEQP-VK.*\"]\n",
            "a `name` of letters",
        ),
        (
            "[[group]]\nname = \"triage\"\ncases = [\"dEQP-VK.*\"]\n",
            "taken by the triage",
        ),
        (
            "[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\nfraction = 0\n",
            "`fraction` must be a positive integer",
        ),
        (
            "[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\nissues = [0]\n",
            "issue numbers",
        ),
        (
            "[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\nlist = 1\n",
            "unknown key `list`",
        ),
        ("groups = 1\n", "unknown key `groups`"),
        (
            "[[group]]\nname = \"g\"\ncases = [\"dEQP-VK.*\"]\n[[group]]\nname = \"g\"\ncases = \
             [\"dEQP-VK.*\"]\n",
            "group g twice",
        ),
    ] {
        let e = Groups::parse(text).err().unwrap();
        assert!(e.contains(error), "{text:?}: {e}");
    }
}

#[test]
fn select_names_groups_or_takes_all() {
    let groups = Groups::builtin().unwrap();
    assert_eq!(groups.select(&[]).unwrap().len(), groups.groups.len());
    let one = groups.select(&[String::from("draw")]).unwrap();
    assert_eq!(one.len(), 1);
    let e = groups.select(&[String::from("nope")]).err().unwrap();
    assert!(e.contains("unknown deqp-vk group \"nope\""), "{e}");
}
