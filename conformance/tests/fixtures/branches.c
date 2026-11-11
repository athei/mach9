/* Synthetic unit: child entry points whose value is returned, setup calls,
 * a function-like macro of the file called as a statement, calls with
 * arguments, repeated calls, calls inside if and else, an #if/#else in the
 * body, a label, and a designated initializer. */

#define check_last_error(expected) check_last_error_(__LINE__, expected)

static void check_last_error_(unsigned int line, DWORD expected)
{
}

static void pump_messages(void)
{
}

static void load_entry_points(void)
{
}

static void test_draw_into_bitmap( BOOL with_alpha )
{
}

static void test_surface( HWND window, HDC dc )
{
}

static void test_format_flag( DWORD flag )
{
}

static void test_read_surface( HDC dc )
{
}

static void test_drop_read_surface( HDC dc )
{
}

static void test_offscreen( HDC dc )
{
}

static void test_swap_interval( HDC dc )
{
}

static void test_manual_window( HWND window, const struct surface_desc *desc )
{
}

static void test_remote_window_child( HWND window, int format )
{
}

static void test_remote_owner_child( HWND window )
{
}

START_TEST(branches)
{
    const struct surface_desc desc =
    {
        .size = sizeof(struct surface_desc),
        .depth = 32,
        .flags = SURFACE_WINDOW | SURFACE_DOUBLE,
    };
    char **args;
    int count = winetest_get_mainargs( &args );
    const char *features;
    HWND window;
    HDC dc;

    if (count >= 4 && !strcmp( args[2], "remote_window" ))
        return test_remote_window_child( (HWND)(ULONG_PTR)strtoul( args[3], NULL, 16 ), atoi( args[4] ) );
    if (count >= 4 && !strcmp( args[2], "remote_owner" ))
        return test_remote_owner_child( (HWND)(ULONG_PTR)strtoul( args[3], NULL, 16 ) );

    window = CreateWindowA( "button", "surface", WS_POPUP | WS_VISIBLE, 0, 0, 64, 64, NULL, NULL,
                            NULL, NULL );
    ok( window != NULL, "no window\n" );
    pump_messages();

    check_last_error( ERROR_SUCCESS );

    if (!(dc = GetDC( window )))
    {
        skip( "No device context.\n" );
        goto done;
    }

    test_draw_into_bitmap( TRUE );
    test_draw_into_bitmap( FALSE );
    test_surface( window, dc );

    load_entry_points();

    test_format_flag( SURFACE_WINDOW );
    test_format_flag( SURFACE_DOUBLE );

    features = get_surface_features( dc );
    if (strstr( features, "read_surface" ))
    {
        test_read_surface( dc );
        test_drop_read_surface( dc );
    }
    else skip( "Reading surfaces is not supported.\n" );

    if (strstr( features, "offscreen" )) test_offscreen( dc );
    else skip( "Offscreen surfaces are not supported.\n" );

#if HAVE_SWAP_INTERVAL
    test_swap_interval( dc );
#else
    skip( "Built without swap intervals.\n" );
#endif

    if (winetest_interactive) test_manual_window( window, &desc );

done:
    ReleaseDC( window, dc );
    DestroyWindow( window );
}
