/* Synthetic unit: one queue helper, two helpers that run the same test
 * function with a key prefix each, direct calls between them, and an early
 * return after a skip. */

static void enqueue(void (*fn)(void))
{
}

static void drain_queue(void)
{
}

static void run_on_old_api(void (*fn)(IUnknown *api, BOOL is_new))
{
}

static void run_on_new_api(void (*fn)(IUnknown *api, BOOL is_new))
{
}

static void test_list_screens(void)
{
}

static void test_screen_modes(void)
{
}

static void test_gamma_ramp(void)
{
}

static void test_open_window(IUnknown *api, BOOL is_new)
{
}

static void test_wide_colours(IUnknown *api, BOOL is_new)
{
}

START_TEST(prefixed)
{
    HMODULE newer;

    ok(GetSystemMetrics(SM_CMONITORS) > 0, "No screen is attached.\n");

    enqueue(test_list_screens);

    drain_queue();

    /* Full-screen tests; they must not overlap. */
    test_screen_modes();
    test_gamma_ramp();
    run_on_old_api(test_open_window);

    if (!(newer = LoadLibraryA("newer_api.dll")))
    {
        skip("The newer API is missing.\n");
        return;
    }

    run_on_new_api(test_open_window);
    run_on_new_api(test_wide_colours);

    FreeLibrary(newer);
}
