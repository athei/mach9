/* Synthetic unit: tests queued through helpers, one helper taking the test
 * function as its third argument, a child entry point followed by return,
 * setup calls, a multi-line call, a function pointer variable, a function
 * pointer array, and direct calls after the queue runs. */

static BOOL slow_mode;

static void enqueue(void (*fn)(void))
{
}

static void enqueue_for_each_mode(void (*fn)(unsigned int mode))
{
}

static void enqueue_for_each_small_mode(void (*fn)(unsigned int mode))
{
}

static void enqueue_for_each_mode_between(unsigned int low,
        unsigned int high, void (*fn)(unsigned int mode))
{
}

static void drain_queue(void)
{
}

static void show_platform_info(void)
{
}

static void test_spawned_child(void)
{
}

static void test_open_widget(void)
{
}

static void test_widget_modes(unsigned int mode)
{
}

static void test_small_widget(unsigned int mode)
{
}

static void test_widget_borders(unsigned int mode)
{
}

static void test_widget_margins(unsigned int mode)
{
}

static void test_serial_paint(void)
{
}

static void test_serial_resize(void)
{
}

static void test_table_one(void)
{
}

static void test_table_two(void)
{
}

static void (*const table_tests[])(void) = {test_table_one, test_table_two};

START_TEST(queued)
{
    BOOL (WINAPI *probe)(DWORD flags);
    unsigned int n, count;
    char **args;

    count = winetest_get_mainargs(&args);
    if (count == 3 && !strcmp(args[2], "spawned"))
    {
        test_spawned_child();
        return;
    }

    if ((probe = (void *)GetProcAddress(GetModuleHandleA("kernel32"), "ProbeFeature"))
            && probe(0))
        slow_mode = TRUE;

    for (n = 2; n < count; ++n)
    {
        if (!strcmp(args[n], "--slow"))
            slow_mode = TRUE;
    }

    show_platform_info();

    enqueue(test_open_widget);
    enqueue_for_each_mode(test_widget_modes);
    enqueue_for_each_small_mode(test_small_widget);
    enqueue_for_each_mode_between(2, 7,
            test_widget_borders);
    enqueue_for_each_mode_between(2, 7, test_widget_margins);

    drain_queue();

    for (n = 0; n < ARRAY_SIZE(table_tests); ++n)
        table_tests[n]();

    /* These two share state with the queued ones, so they run after them. */
    test_serial_paint();
    test_serial_resize();
}
