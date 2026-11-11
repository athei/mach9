use super::*;
use crate::model::{Dispatch, Selection, Unit, Wrapping};

const QUEUED: &str = include_str!("../../tests/fixtures/queued.c");
const PREFIXED: &str = include_str!("../../tests/fixtures/prefixed.c");
const BRANCHES: &str = include_str!("../../tests/fixtures/branches.c");
const THIRD_ARG: &str = include_str!("../../tests/fixtures/third_arg.c");
const BACKENDS: &str = include_str!("../../tests/fixtures/backends.c");
const ANY_NAME: &str = include_str!("../../tests/fixtures/any_name.c");

fn helper(name: &str) -> Dispatch {
    Dispatch {
        name: name.to_owned(),
        args: 1,
        key: 1,
        prefix: String::new(),
    }
}

fn helper_at(name: &str, args: usize, key: usize) -> Dispatch {
    Dispatch {
        args,
        key,
        ..helper(name)
    }
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn wrapped(name: &str, dispatch: Vec<Dispatch>, setup: &[&str]) -> Unit {
    with_tests(name, dispatch, setup, &[])
}

fn with_tests(name: &str, dispatch: Vec<Dispatch>, setup: &[&str], tests: &[&str]) -> Unit {
    Unit {
        name: name.to_owned(),
        file: format!("{name}.c"),
        selection: Selection::Wrapped(Wrapping {
            dispatch,
            setup: names(setup),
            tests: names(tests),
        }),
    }
}

fn whole(name: &str) -> Unit {
    Unit {
        name: name.to_owned(),
        file: format!("{name}.c"),
        selection: Selection::Whole,
    }
}

fn queued_dispatch() -> Vec<Dispatch> {
    vec![
        helper("enqueue"),
        helper("enqueue_for_each_mode"),
        helper("enqueue_for_each_small_mode"),
        helper_at("enqueue_for_each_mode_between", 3, 3),
    ]
}

fn queued() -> Unit {
    wrapped(
        "queued",
        queued_dispatch(),
        &["test_spawned_child", "show_platform_info", "drain_queue"],
    )
}

fn prefixed() -> Unit {
    wrapped(
        "prefixed",
        vec![
            helper("enqueue"),
            Dispatch {
                prefix: String::from("old:"),
                ..helper("run_on_old_api")
            },
            Dispatch {
                prefix: String::from("new:"),
                ..helper("run_on_new_api")
            },
        ],
        &["drain_queue"],
    )
}

fn branches() -> Unit {
    wrapped(
        "branches",
        Vec::new(),
        &[
            "test_remote_window_child",
            "test_remote_owner_child",
            "pump_messages",
            "load_entry_points",
            "check_last_error",
        ],
    )
}

fn third_arg() -> Unit {
    wrapped(
        "third_arg",
        vec![
            helper("for_each_adapter"),
            helper_at("for_each_adapter_with", 4, 3),
        ],
        &[],
    )
}

fn backends() -> Unit {
    wrapped(
        "backends",
        vec![helper("run_for_each_backend")],
        &["create_context", "query_identity", "start_focus_thread"],
    )
}

fn any_name() -> Unit {
    wrapped("any_name", Vec::new(), &[])
}

/// Plan `unit` on `text`: the plan, or every error.
fn run(unit: &Unit, text: &str) -> Result<Plan, Vec<String>> {
    let mut errors = Vec::new();
    match plan_source("exe", unit, text.to_owned(), &mut errors) {
        Some(plan) if errors.is_empty() => Ok(plan),
        _ => Err(errors),
    }
}

/// The errors of planning `unit` on `text`, which must fail.
fn errors_of(unit: &Unit, text: &str) -> Vec<String> {
    match run(unit, text) {
        Ok(_) => panic!("planning {} succeeded", unit.name),
        Err(errors) => errors,
    }
}

fn keys(plan: &Plan) -> Vec<&str> {
    plan.keys
        .keys
        .as_ref()
        .expect("a wrapped unit has keys")
        .iter()
        .map(String::as_str)
        .collect()
}

/// The generated block of a plan, from its first to its last line.
fn block(plan: &Plan) -> &str {
    let start = plan.updated.find(BEGIN).expect("a block");
    let end = plan.updated.find(END).expect("a block") + END.len();
    &plan.updated[start..end]
}

/// The 1-based line of the first line of `text` holding `needle`.
fn line_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|line| line.contains(needle))
        .expect("the needle is in the text")
        + 1
}

fn has_error(errors: &[String], expected: &str) -> bool {
    errors.iter().any(|error| error.contains(expected))
}

#[test]
fn queued_keys_follow_the_body_and_leave_setup_unwrapped() {
    let plan = run(&queued(), QUEUED).unwrap();
    assert_eq!(
        keys(&plan),
        [
            "test_open_widget",
            "test_widget_modes",
            "test_small_widget",
            "test_widget_borders",
            "test_widget_margins",
            "test_serial_paint",
            "test_serial_resize",
        ]
    );
    let block = block(&plan);
    assert!(block.contains(
        "#define enqueue_for_each_mode_between(a1, a2, f) do { if (winetest_selected(#f)) \
         enqueue_for_each_mode_between(a1, a2, f); } while (0)"
    ));
    assert!(block.contains(
        "#define test_serial_paint(...) do { if (winetest_selected(\"test_serial_paint\")) \
         test_serial_paint(__VA_ARGS__); } while (0)"
    ));
    for setup in ["test_spawned_child", "show_platform_info", "drain_queue"] {
        assert!(!block.contains(&format!("#define {setup}(")), "{setup}");
    }
}

/// A call through a function pointer variable or an element of a function
/// pointer array is not a call of a local function: no key, no error.
#[test]
fn calls_through_function_pointers_are_neither_keys_nor_errors() {
    let plan = run(&queued(), QUEUED).unwrap();
    let keys = keys(&plan);
    for name in ["probe", "table_tests", "test_table_one", "test_table_two"] {
        assert!(!keys.contains(&name), "{name}: {keys:?}");
        assert!(
            !block(&plan).contains(&format!("#define {name}(")),
            "{name}"
        );
    }
}

#[test]
fn the_block_is_an_addition_right_before_start_test() {
    let plan = run(&queued(), QUEUED).unwrap();
    let start = plan.updated.find(BEGIN).unwrap();
    let start_test = plan.updated.find("START_TEST(queued)").unwrap();
    assert!(plan.updated[..start_test].ends_with(&format!("{END}\n\n")));
    let without = format!("{}{}", &plan.updated[..start], &plan.updated[start_test..]);
    assert_eq!(without, QUEUED);
}

#[test]
fn keys_of_two_helpers_carry_their_prefix() {
    let plan = run(&prefixed(), PREFIXED).unwrap();
    assert_eq!(
        keys(&plan),
        [
            "test_list_screens",
            "test_screen_modes",
            "test_gamma_ramp",
            "old:test_open_window",
            "new:test_open_window",
            "new:test_wide_colours",
        ]
    );
    assert!(block(&plan).contains(
        "#define run_on_new_api(f) do { if (winetest_selected(\"new:\" #f)) run_on_new_api(f); \
         } while (0)"
    ));
}

#[test]
fn calls_in_if_and_else_are_wrapped_and_repeated_calls_once() {
    let plan = run(&branches(), BRANCHES).unwrap();
    assert_eq!(
        keys(&plan),
        [
            "test_draw_into_bitmap",
            "test_surface",
            "test_format_flag",
            "test_read_surface",
            "test_drop_read_surface",
            "test_offscreen",
            "test_swap_interval",
            "test_manual_window",
        ]
    );
    let block = block(&plan);
    assert_eq!(block.matches("#define test_draw_into_bitmap(").count(), 1);
    assert!(!block.contains("check_last_error"));
}

#[test]
fn a_call_whose_value_is_used_is_never_wrapped() {
    let unit = wrapped(
        "branches",
        Vec::new(),
        &["pump_messages", "load_entry_points", "check_last_error"],
    );
    let errors = errors_of(&unit, BRANCHES);
    for name in ["test_remote_window_child", "test_remote_owner_child"] {
        let line = line_of(BRANCHES, &format!("return {name}("));
        assert!(
            has_error(
                &errors,
                &format!("branches.c:{line}: {name}(): neither wrapped nor setup")
            ),
            "{errors:?}"
        );
    }
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn a_helper_keys_on_its_third_argument() {
    let plan = run(&third_arg(), THIRD_ARG).unwrap();
    assert_eq!(
        keys(&plan),
        [
            "test_shared_buffers",
            "test_version_number",
            "list_adapter_outputs",
            "test_window_target",
        ]
    );
    assert!(block(&plan).contains(
        "#define for_each_adapter_with(a1, a2, f, a4) do { if (winetest_selected(#f)) \
         for_each_adapter_with(a1, a2, f, a4); } while (0)"
    ));
}

#[test]
fn a_dispatch_call_with_other_arguments_than_configured_is_an_error() {
    let unit = wrapped(
        "third_arg",
        vec![
            helper("for_each_adapter"),
            helper_at("for_each_adapter_with", 3, 3),
        ],
        &[],
    );
    let errors = errors_of(&unit, THIRD_ARG);
    assert!(
        has_error(&errors, "called with 4 arguments, wine-tests.toml says 3"),
        "{errors:?}"
    );
}

#[test]
fn a_helper_runs_tests_per_backend_and_callbacks_are_no_keys() {
    let plan = run(&backends(), BACKENDS).unwrap();
    assert_eq!(
        keys(&plan),
        [
            "test_transform_points",
            "test_depth_copy",
            "test_depth_off",
            "test_exclusive_clipping",
        ]
    );
    assert!(!block(&plan).contains("count_backend"));
}

#[test]
fn an_unlisted_local_call_is_rejected_with_its_file_and_line() {
    let unit = wrapped(
        "backends",
        vec![helper("run_for_each_backend")],
        &["query_identity", "start_focus_thread"],
    );
    let errors = errors_of(&unit, BACKENDS);
    let line = line_of(BACKENDS, "if (!(context = create_context()))");
    assert_eq!(
        errors,
        [format!(
            "backends.c:{line}: create_context(): neither wrapped nor setup: the call's value is \
             used, so it cannot be wrapped; name it as setup"
        )]
    );
}

#[test]
fn a_helper_that_takes_a_local_function_must_be_configured() {
    let unit = wrapped(
        "backends",
        Vec::new(),
        &["create_context", "query_identity", "start_focus_thread"],
    );
    let errors = errors_of(&unit, BACKENDS);
    assert!(
        has_error(
            &errors,
            "run_for_each_backend(): neither wrapped nor setup: it takes the local function \
             test_depth_copy"
        ),
        "{errors:?}"
    );
}

#[test]
fn test_functions_named_test_star_and_star_test_are_wrapped() {
    let plan = run(&any_name(), ANY_NAME).unwrap();
    assert_eq!(
        keys(&plan),
        [
            "test_fpu_state",
            "test_list_formats",
            "clear_blit_test",
            "test_sampler_modes",
        ]
    );
}

#[test]
fn write_is_idempotent() {
    for (unit, text) in [
        (queued(), QUEUED),
        (prefixed(), PREFIXED),
        (branches(), BRANCHES),
        (third_arg(), THIRD_ARG),
        (backends(), BACKENDS),
        (any_name(), ANY_NAME),
    ] {
        let first = run(&unit, text).unwrap();
        let stale = first.stale.as_deref().unwrap_or_default();
        assert!(stale.contains("no generated block"), "{stale}");
        let second = run(&unit, &first.updated).unwrap();
        assert!(second.stale.is_none(), "{:?}", second.stale);
        assert_eq!(second.updated, first.updated);
        assert_eq!(second.keys.keys, first.keys.keys);
    }
}

#[test]
fn an_out_of_date_block_is_reported_and_replaced() {
    let fresh = run(&queued(), QUEUED).unwrap().updated;
    let edited = fresh.replacen("#define test_serial_resize", "#define test_old_name", 1);
    let plan = run(&queued(), &edited).unwrap();
    let stale = plan.stale.unwrap();
    assert!(
        stale.contains("differs from what --write generates"),
        "{stale}"
    );
    assert_eq!(plan.updated, fresh);
}

#[test]
fn a_whole_unit_has_no_block() {
    let with_block = run(&queued(), QUEUED).unwrap().updated;
    let plan = run(&whole("queued"), &with_block).unwrap();
    let stale = plan.stale.unwrap();
    assert!(
        stale.contains("runs as a whole but has a generated block"),
        "{stale}"
    );
    assert_eq!(plan.updated, QUEUED);
    assert!(plan.keys.keys.is_none());

    let plan = run(&whole("queued"), QUEUED).unwrap();
    assert!(plan.stale.is_none());
}

#[test]
fn an_end_marker_without_its_beginning_is_an_error() {
    let text = QUEUED.replace("START_TEST(queued)", &format!("{END}\nSTART_TEST(queued)"));
    let errors = errors_of(&queued(), &text);
    assert!(has_error(&errors, "without its beginning"), "{errors:?}");
}

#[test]
fn setup_and_helpers_the_body_does_not_call_are_errors() {
    let mut dispatch = queued_dispatch();
    dispatch.push(helper("enqueue_missing"));
    let unit = wrapped(
        "queued",
        dispatch,
        &[
            "test_spawned_child",
            "show_platform_info",
            "drain_queue",
            "init_missing",
        ],
    );
    let errors = errors_of(&unit, QUEUED);
    assert!(
        has_error(&errors, "setup init_missing is not a local function"),
        "{errors:?}"
    );
    assert!(
        has_error(
            &errors,
            "dispatch helper enqueue_missing is not a local function"
        ),
        "{errors:?}"
    );
}

#[test]
fn a_shadowed_call_after_start_test_is_an_error() {
    let text = format!("{QUEUED}\nstatic void later(void)\n{{\n    test_serial_resize();\n}}\n");
    let errors = errors_of(&queued(), &text);
    assert!(
        has_error(&errors, "test_serial_resize() after START_TEST(queued)"),
        "{errors:?}"
    );
}

#[test]
fn a_missing_start_test_is_an_error() {
    let errors = errors_of(&wrapped("other", Vec::new(), &[]), QUEUED);
    assert!(has_error(&errors, "no START_TEST(other)"), "{errors:?}");
}

/// Calls in comments, literals and directives are not calls, and a call may
/// span lines.
const LEXING: &str = r#"
static void test_a(void) {}
static void test_b(int x) {}
static int test_c(void) { return 0; }

START_TEST(lexing)
{
    /* test_b(1); */
    // test_b(2);
    trace("test_b(3);\n");
    trace("%c%c", ')', '"');
#define CALL_B(x) \
    test_b(x)
    test_a(
        );
    switch (argc)
    {
    case 1: test_b(4); break;
    default: test_a(); break;
    }
    if (test_c()) {}
}
"#;

#[test]
fn comments_literals_and_directives_hold_no_calls() {
    let unit = wrapped("lexing", Vec::new(), &["test_c"]);
    let plan = run(&unit, LEXING).unwrap();
    assert_eq!(keys(&plan), ["test_a", "test_b"]);
}

#[test]
fn a_call_in_a_condition_or_initializer_is_value_used() {
    let text = LEXING.replace("if (test_c()) {}", "int r = test_c();");
    let errors = errors_of(&wrapped("lexing", Vec::new(), &[]), &text);
    assert!(
        has_error(&errors, "test_c(): neither wrapped nor setup"),
        "{errors:?}"
    );
    let errors = errors_of(&wrapped("lexing", Vec::new(), &[]), LEXING);
    assert!(
        has_error(&errors, "test_c(): neither wrapped nor setup"),
        "{errors:?}"
    );
}

/// Escapes inside literals neither end them early nor hide a call after
/// them.
#[test]
fn escapes_in_literals_do_not_end_them() {
    let text = r#"
static void test_a(void) {}
static void test_b(void) {}

START_TEST(escapes)
{
    trace("a\"b test_b(); \\");
    trace("%c", '\'');
    trace("%c", '\\');
    trace("tab\t\"test_b()\"\n");
    test_a();
}
"#;
    let plan = run(&wrapped("escapes", Vec::new(), &[]), text).unwrap();
    assert_eq!(keys(&plan), ["test_a"]);
}

/// No input makes the lexer or the scan panic: every prefix of the
/// fixtures, which cuts literals, comments, directives and brackets, and
/// literals that end in a backslash at the end of the input.
#[test]
fn no_input_panics() {
    for text in [QUEUED, BRANCHES, LEXING] {
        for (cut, _) in text.char_indices() {
            let mut errors = Vec::new();
            let _ = plan_source("exe", &queued(), text[..cut].to_owned(), &mut errors);
        }
    }
    for text in [
        "\"\\",
        "'\\",
        "\"abc\\",
        "#define X \"\\",
        "/*",
        "#",
        "#define",
        "\\",
        "\"é\\",
        "-",
        "f(",
        ")",
        "{",
        "#if",
        "#else",
        "#endif",
        "START_TEST(",
    ] {
        let lexed = super::lex::Lexed::new(text);
        for index in 0..=lexed.tokens.len() {
            let _ = lexed.text(index);
        }
        let mut errors = Vec::new();
        let _ = plan_source("exe", &queued(), text.to_owned(), &mut errors);
    }
}

/// Both branches of an `#if` in a body are scanned when each leaves the
/// same brackets open.
#[test]
fn both_branches_of_an_if_in_a_body_are_scanned() {
    let text = r"
static void test_a(void) {}
static void test_b(void) {}
static void test_c(void) {}

START_TEST(branches)
{
#if VERSION
    if (x) test_a();
#elif OTHER
    {
        test_b();
    }
#else
    test_b();
#endif
    test_c();
}
";
    let plan = run(&wrapped("branches", Vec::new(), &[]), text).unwrap();
    assert_eq!(keys(&plan), ["test_a", "test_b", "test_c"]);
}

/// Each branch of an `#if` starts with the brackets open before it, so a
/// body that each branch closes on its own is matched in both.
#[test]
fn each_branch_of_an_if_starts_with_the_brackets_open_before_it() {
    let text = r"
static void test_a(void) {}
static void test_b(void) {}

START_TEST(branches)
{
    test_a();
#if VERSION
    test_b();
}
#else
}
#endif
";
    let plan = run(&wrapped("branches", Vec::new(), &[]), text).unwrap();
    assert_eq!(keys(&plan), ["test_a", "test_b"]);
}

#[test]
fn unmatched_brackets_are_errors() {
    for (text, expected) in [
        (
            "START_TEST(x)\n{\n    f(];\n}\n",
            "3: `]` closes the `(` of line 3",
        ),
        ("START_TEST(x)\n{\n    f();\n", "2: `{` is never closed"),
        ("START_TEST(x)\n{\n}\n}\n", "4: `}` with no bracket open"),
        (
            "#if A\nstatic void f(int a) {\n#else\nstatic void f(void) {\n#endif\n}\n",
            "1: the branches of this #if leave different brackets open, so the scan cannot \
             follow them",
        ),
        ("#endif\n", "1: #endif without #if"),
        ("#else\n", "1: #else or #elif without #if"),
        ("#ifdef A\n", "1: #if without #endif"),
    ] {
        let errors = errors_of(&wrapped("x", Vec::new(), &[]), text);
        assert_eq!(errors, [format!("x.c:{expected}")], "{text}");
    }
}

#[test]
fn a_dispatch_key_must_be_a_local_function() {
    for argument in ["NULL", "callback"] {
        let text = THIRD_ARG.replace(
            "for_each_adapter(list_adapter_outputs);",
            &format!("for_each_adapter({argument});"),
        );
        let line = line_of(&text, &format!("for_each_adapter({argument});"));
        let errors = errors_of(&third_arg(), &text);
        assert_eq!(
            errors,
            [format!(
                "third_arg.c:{line}: for_each_adapter(): argument 1 is not a function the file \
                 defines, so it cannot be the key"
            )]
        );
    }
}

/// A local call that is not named like a test is an error until the config
/// names it.
#[test]
fn a_direct_call_must_look_like_a_test_or_be_listed() {
    let text = ANY_NAME.replace(
        "static void test_sampler_modes(void)",
        "static void check_sampler_modes(void)",
    );
    let text = text.replace("    test_sampler_modes();", "    check_sampler_modes();");
    let line = line_of(&text, "    check_sampler_modes();");
    let errors = errors_of(&any_name(), &text);
    assert_eq!(
        errors,
        [format!(
            "any_name.c:{line}: check_sampler_modes(): neither wrapped nor setup: the name is \
             not test_* or *_test; name it in `tests` or as setup"
        )]
    );
    let unit = with_tests("any_name", Vec::new(), &[], &["check_sampler_modes"]);
    let plan = run(&unit, &text).unwrap();
    assert_eq!(keys(&plan).last(), Some(&"check_sampler_modes"));
    assert!(block(&plan).contains(&direct_macro("check_sampler_modes")));
}

/// A call followed by `return;` is a child process entry point until the
/// config names it.
#[test]
fn a_call_followed_by_return_must_be_listed() {
    let setup = &["show_platform_info", "drain_queue"];
    let line = line_of(QUEUED, "        test_spawned_child();");
    let errors = errors_of(&wrapped("queued", queued_dispatch(), setup), QUEUED);
    assert_eq!(
        errors,
        [format!(
            "queued.c:{line}: test_spawned_child(): neither wrapped nor setup: `return;` \
             follows the call, the shape of a child process entry point; name it as setup, or \
             in `tests` if it is a test"
        )]
    );
    let unit = with_tests("queued", queued_dispatch(), setup, &["test_spawned_child"]);
    let plan = run(&unit, QUEUED).unwrap();
    assert_eq!(keys(&plan)[0], "test_spawned_child");
}

#[test]
fn a_listed_test_the_body_does_not_call_is_an_error() {
    let unit = with_tests("any_name", Vec::new(), &[], &["test_missing"]);
    let errors = errors_of(&unit, ANY_NAME);
    assert_eq!(
        errors,
        [
            "any_name.c: test test_missing is not a local function START_TEST(any_name) calls \
             as a statement"
        ]
    );
}

/// A statement call of a function-like macro of the file is an error until
/// it is setup; the macros of the generated block do not count.
#[test]
fn a_macro_of_the_file_called_in_the_body_must_be_setup() {
    let unit = wrapped(
        "branches",
        Vec::new(),
        &[
            "test_remote_window_child",
            "test_remote_owner_child",
            "pump_messages",
            "load_entry_points",
        ],
    );
    let line = line_of(BRANCHES, "    check_last_error( ERROR_SUCCESS );");
    let errors = errors_of(&unit, BRANCHES);
    assert_eq!(
        errors,
        [format!(
            "branches.c:{line}: check_last_error(): neither wrapped nor setup: a function-like \
             macro of the file, which may run a test that no key selects; name it as setup"
        )]
    );
    let first = run(&branches(), BRANCHES).unwrap();
    let second = run(&branches(), &first.updated).unwrap();
    assert!(second.stale.is_none());
}

/// A Wine source tree under the target directory, removed when dropped.
struct Tree {
    root: std::path::PathBuf,
}

impl Tree {
    fn new(name: &str) -> Self {
        let exe = std::env::current_exe().unwrap();
        let root = exe
            .parent()
            .unwrap()
            .join("conformance-test-trees")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, file: &str, text: &str) {
        let path = self.root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, file: &str) -> String {
        fs::read_to_string(self.root.join(file)).unwrap()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const MAKEFILE: &str = "TESTDLL = x.dll\nPARENTSRC = ../../y/tests\n\nSOURCES = \\\n\tany_name.c \\\n\
                        \twhole.c \\\n\tthird_arg.c \\\n\tresource.rc\n";
const WHOLE: &str = "static void helper(void) {}\n\nSTART_TEST(whole)\n{\n    helper();\n}\n";
const CONFIG: &str = r#"
[[exe]]
name = "x"
dir = "dlls/x/tests"

[[exe.unit]]
name = "any_name"

[[exe.unit]]
name = "whole"
whole = true

[[exe.unit]]
name = "third_arg"
file = "dlls/y/tests/third_arg.c"
dispatch = [{ name = "for_each_adapter" }, { name = "for_each_adapter_with", args = 4, key = 3 }]
"#;

/// A tree with exe x: `any_name.c` and `whole.c` in its directory, and
/// `third_arg.c` through `PARENTSRC`.
fn tree(name: &str) -> Tree {
    let tree = Tree::new(name);
    tree.write("dlls/x/tests/Makefile.in", MAKEFILE);
    tree.write("dlls/x/tests/any_name.c", ANY_NAME);
    tree.write("dlls/x/tests/whole.c", WHOLE);
    tree.write("dlls/y/tests/third_arg.c", THIRD_ARG);
    tree
}

#[test]
fn write_then_check_a_tree() {
    let tree = tree("write-then-check");
    let config = Config::parse(CONFIG).unwrap();
    let Err(errors) = check(&config, &tree.root) else {
        panic!("a tree without blocks passed the check");
    };
    let any_line = line_of(ANY_NAME, "START_TEST(any_name)");
    let third_line = line_of(THIRD_ARG, "START_TEST(third_arg)");
    assert_eq!(
        errors,
        [
            format!(
                "dlls/x/tests/any_name.c:{any_line}: no generated block before \
                 START_TEST(any_name)"
            ),
            format!(
                "dlls/y/tests/third_arg.c:{third_line}: no generated block before \
                 START_TEST(third_arg)"
            ),
        ]
    );

    let written = write(&config, &tree.root).unwrap();
    assert_eq!(
        written,
        ["dlls/x/tests/any_name.c", "dlls/y/tests/third_arg.c"]
    );
    assert_eq!(tree.read("dlls/x/tests/whole.c"), WHOLE);
    let any = tree.read("dlls/x/tests/any_name.c");
    let block_start = any.find(BEGIN).unwrap();
    let start_test = any.find("START_TEST(any_name)").unwrap();
    assert_eq!(
        &any[block_start..start_test],
        format!(
            "{BEGIN}\n{}\n{}\n{}\n{}\n{END}\n\n",
            direct_macro("test_fpu_state"),
            direct_macro("test_list_formats"),
            direct_macro("clear_blit_test"),
            direct_macro("test_sampler_modes"),
        )
    );
    assert_eq!(
        format!("{}{}", &any[..block_start], &any[start_test..]),
        ANY_NAME
    );

    let units = check(&config, &tree.root).unwrap();
    let listed: Vec<(&str, Option<usize>)> = units
        .iter()
        .map(|unit| (unit.unit.as_str(), unit.keys.as_ref().map(Vec::len)))
        .collect();
    assert_eq!(
        listed,
        [
            ("any_name", Some(4)),
            ("whole", None),
            ("third_arg", Some(4))
        ]
    );

    assert_eq!(write(&config, &tree.root).unwrap(), Vec::<String>::new());
    assert_eq!(tree.read("dlls/x/tests/any_name.c"), any);

    tree.write("dlls/x/tests/any_name.c", ANY_NAME);
    let Err(errors) = check(&config, &tree.root) else {
        panic!("a removed block passed the check");
    };
    assert_eq!(
        errors,
        [format!(
            "dlls/x/tests/any_name.c:{any_line}: no generated block before START_TEST(any_name)"
        )]
    );
}

#[test]
fn every_start_test_of_the_makefile_sources_needs_a_unit() {
    let tree = tree("unlisted-units");
    tree.write(
        "dlls/x/tests/Makefile.in",
        &MAKEFILE.replace("\tresource.rc", "\textra.c \\\n\tresource.rc"),
    );
    tree.write(
        "dlls/x/tests/extra.c",
        WHOLE.replace("whole", "extra").as_str(),
    );
    tree.write("dlls/x/tests/unbuilt.c", &WHOLE.replace("whole", "unbuilt"));
    let config = CONFIG.replace(
        "[[exe.unit]]\nname = \"third_arg\"",
        "[[exe.unit]]\nname = \"unbuilt\"\nwhole = true\n\n[[exe.unit]]\nname = \"other\"",
    );
    let config = Config::parse(&config).unwrap();
    let Err(errors) = plan(&config, &tree.root) else {
        panic!("unlisted units passed");
    };
    for expected in [
        "dlls/y/tests/third_arg.c: START_TEST(third_arg) has no unit under exe x in \
         wine-tests.toml",
        "dlls/x/tests/extra.c: START_TEST(extra) has no unit under exe x in wine-tests.toml",
        "dlls/x/tests/Makefile.in: its SOURCES do not build START_TEST(unbuilt) from \
         dlls/x/tests/unbuilt.c, the file of unit x/unbuilt",
    ] {
        assert!(has_error(&errors, expected), "{expected}: {errors:?}");
    }
}

#[test]
fn a_source_the_makefile_cannot_find_is_an_error() {
    let tree = tree("missing-source");
    fs::remove_file(tree.root.join("dlls/y/tests/third_arg.c")).unwrap();
    let config = Config::parse(CONFIG).unwrap();
    let Err(errors) = plan(&config, &tree.root) else {
        panic!("a missing source passed");
    };
    assert!(
        has_error(
            &errors,
            "dlls/x/tests/Makefile.in: third_arg.c of SOURCES is in none of dlls/x/tests, \
             dlls/y/tests"
        ),
        "{errors:?}"
    );
}

#[test]
fn units_sharing_a_file_must_generate_the_same_block() {
    let tree = tree("shared-file");
    tree.write(
        "dlls/z/tests/Makefile.in",
        "PARENTSRC = ../../y/tests\nSOURCES = third_arg.c\n",
    );
    let shared = r#"
[[exe]]
name = "z"
dir = "dlls/z/tests"

[[exe.unit]]
name = "third_arg"
file = "dlls/y/tests/third_arg.c"
dispatch = [{ name = "for_each_adapter", prefix = "z:" }, { name = "for_each_adapter_with", args = 4, key = 3 }]
"#;
    let config = Config::parse(&format!("{CONFIG}{shared}")).unwrap();
    let Err(errors) = plan(&config, &tree.root) else {
        panic!("differing blocks for one file passed");
    };
    assert_eq!(
        errors,
        [
            "dlls/y/tests/third_arg.c: units x/third_arg and z/third_arg share the file but \
             generate different blocks"
        ]
    );
    let same = shared.replace(", prefix = \"z:\"", "");
    let config = Config::parse(&format!("{CONFIG}{same}")).unwrap();
    assert!(plan(&config, &tree.root).is_ok());
}

#[test]
fn join_relative_resolves_parent_directories() {
    assert_eq!(
        join_relative("dlls/a/tests", "../../b/tests"),
        "dlls/b/tests"
    );
    assert_eq!(join_relative("dlls/x/tests", "./sub"), "dlls/x/tests/sub");
}
