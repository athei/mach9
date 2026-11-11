/* Synthetic unit: setup calls whose value is used, a callback passed to a
 * function the file does not define, a setup thread, a helper that runs a
 * test once per backend, and direct calls. */

static IUnknown *create_context(void)
{
    return NULL;
}

static BOOL query_identity(IUnknown *context, struct identity *identity)
{
    return FALSE;
}

static HRESULT WINAPI count_backend(const char *name, void *user)
{
    return S_OK;
}

static void start_focus_thread(void)
{
}

static void run_for_each_backend(void (*fn)(const GUID *backend))
{
}

static void test_transform_points(void)
{
}

static void test_depth_copy(const GUID *backend)
{
}

static void test_depth_off(const GUID *backend)
{
}

static void test_exclusive_clipping(void)
{
}

START_TEST(backends)
{
    struct identity identity;
    unsigned int backends = 0;
    IUnknown *context;
    HRESULT hr;

    if (!(context = create_context()))
    {
        skip("No context, skipping the unit.\n");
        return;
    }

    if (query_identity(context, &identity))
        trace("Running on \"%s\".\n", identity.name);

    hr = EnumerateBackends(context, count_backend, &backends);
    ok(hr == S_OK, "Got hr %#lx.\n", hr);
    IUnknown_Release(context);

    start_focus_thread();

    test_transform_points();
    run_for_each_backend(test_depth_copy);
    run_for_each_backend(test_depth_off);
    test_exclusive_clipping();
}
