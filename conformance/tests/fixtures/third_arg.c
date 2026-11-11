/* Synthetic unit: a helper with the test function as its only argument, a
 * helper with it as the third of four, a child branch that runs a test
 * through that helper and returns, and direct calls. */

static void for_each_adapter_with(unsigned int count, const char * const *names,
        void (*fn)(HANDLE session, HANDLE adapter), void *context)
{
}

static void for_each_adapter(void (*fn)(HANDLE adapter))
{
}

static void test_version_number(void)
{
}

static void list_adapter_outputs(HANDLE adapter)
{
}

static void test_shared_buffers(HANDLE session, HANDLE adapter)
{
}

static void test_window_target(HANDLE session, HANDLE adapter)
{
}

START_TEST(third_arg)
{
    char **args;
    int count;

    count = winetest_get_mainargs(&args);

    if (count > 3 && !strcmp(args[2], "share"))
    {
        ok(count >= 6, "The child needs more arguments.\n");
        for_each_adapter_with(ARRAY_SIZE(buffer_names), buffer_names, test_shared_buffers, NULL);
        return;
    }

    test_version_number();
    for_each_adapter(list_adapter_outputs);
    for_each_adapter_with(ARRAY_SIZE(window_names), window_names, test_window_target, NULL);
    for_each_adapter_with(ARRAY_SIZE(buffer_names), buffer_names, test_shared_buffers, NULL);
}
