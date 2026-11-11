/* Synthetic unit: direct calls of test functions named test_* and *_test,
 * after setup that returns early on failure. */

static void test_fpu_state(void)
{
}

static void test_list_formats(void)
{
}

static void clear_blit_test(void)
{
}

static void test_sampler_modes(void)
{
}

START_TEST(any_name)
{
    HMODULE library = LoadLibraryA("widgets.dll");
    WNDCLASSA window_class = {0};

    if (!library)
    {
        skip("widgets.dll is missing.\n");
        return;
    }

    window_class.lpfnWndProc = DefWindowProcA;
    window_class.lpszClassName = "any_name_class";
    RegisterClassA(&window_class);

    test_fpu_state();
    test_list_formats();
    clear_blit_test();
    test_sampler_modes();

    UnregisterClassA("any_name_class", GetModuleHandleA(NULL));
    FreeLibrary(library);
}
