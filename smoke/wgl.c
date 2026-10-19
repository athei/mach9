/* The WGL program bundle-wine.sh builds for each PE architecture of
 * SMOKE_PE_ARCHS and runs under the bundled wine (check_wgl_runs). It opens
 * a visible window, sets a double-buffered RGBA pixel format on it and
 * creates three contexts in turn: a legacy one with wglCreateContext, then a
 * core profile context of the highest 4.x version wglCreateContextAttribsARB
 * gives, then a compatibility profile one. For each it prints GL_RENDERER,
 * GL_VERSION and GL_SHADING_LANGUAGE_VERSION, clears the back buffer to a
 * color and reads the center pixel back, and swaps a few frames. With the
 * core context it also times 60 swaps with wglSwapIntervalEXT(1) and 60 with
 * wglSwapIntervalEXT(0) and prints the average frame time of each, so a run
 * shows whether vsync can be turned off; the times only inform, since they
 * depend on the display and on whether the window is visible.
 *
 * With an argument, GL_RENDERER of every context must contain it, so a run
 * can require Zink or Apple's renderer. A core context below 4.0 fails. The
 * exit status names the first step that failed (enum below); each failure
 * also prints a line.
 *
 * llvm-mingw has GL/gl.h for OpenGL 1.1 only, so the few later constants it
 * needs are written out here, with the values of glext.h and wglext.h. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>
#include <GL/gl.h>

#if defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#elif defined(__aarch64__)
#define ARCH "aarch64"
#else
#error "unknown architecture"
#endif

#define GL_SHADING_LANGUAGE_VERSION 0x8B8C
#define WGL_CONTEXT_MAJOR_VERSION_ARB 0x2091
#define WGL_CONTEXT_MINOR_VERSION_ARB 0x2092
#define WGL_CONTEXT_PROFILE_MASK_ARB 0x9126
#define WGL_CONTEXT_CORE_PROFILE_BIT_ARB 0x00000001
#define WGL_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB 0x00000002

typedef HGLRC (WINAPI *PFN_wglCreateContextAttribsARB)(HDC, HGLRC, const int *);
typedef BOOL (WINAPI *PFN_wglSwapIntervalEXT)(int);

enum
{
    FAIL_WINDOW = 1,
    FAIL_PIXEL_FORMAT,
    FAIL_LEGACY_CONTEXT,
    FAIL_MAKE_CURRENT,
    FAIL_STRINGS,
    FAIL_RENDERER,
    FAIL_READBACK,
    FAIL_SWAP,
    FAIL_NO_CREATE_CONTEXT_ATTRIBS,
    FAIL_CORE_CONTEXT,
    FAIL_COMPAT_CONTEXT,
};

static const char *expect_renderer;
static int width, height;

static void pump(void)
{
    MSG msg;

    while (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE))
    {
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
}

/* Clear the back buffer to <r>, <g>, <b> and check the center pixel. */
static int clear_and_check(const char *what, BYTE r, BYTE g, BYTE b)
{
    BYTE px[4] = {0};
    GLenum err;

    glViewport(0, 0, width, height);
    glClearColor(r / 255.0f, g / 255.0f, b / 255.0f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glFinish();
    glReadPixels(width / 2, height / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
    if ((err = glGetError()) != GL_NO_ERROR)
    {
        printf("wine-build smoke test: %s %s glClear/glReadPixels raised GL error %#x\n", ARCH,
               what, err);
        return FAIL_READBACK;
    }
    if (abs(px[0] - r) > 2 || abs(px[1] - g) > 2 || abs(px[2] - b) > 2)
    {
        printf("wine-build smoke test: %s %s read %u,%u,%u back after clearing to %u,%u,%u\n",
               ARCH, what, px[0], px[1], px[2], r, g, b);
        return FAIL_READBACK;
    }
    return 0;
}

/* Make <ctx> current, print its strings, check a clear and swap a few
 * frames. */
static int exercise(const char *what, HDC dc, HGLRC ctx)
{
    const char *renderer, *version, *glsl;
    int i, ret;

    if (!wglMakeCurrent(dc, ctx))
    {
        printf("wine-build smoke test: %s %s wglMakeCurrent failed, error %lu\n", ARCH, what,
               GetLastError());
        return FAIL_MAKE_CURRENT;
    }
    renderer = (const char *)glGetString(GL_RENDERER);
    version = (const char *)glGetString(GL_VERSION);
    glsl = (const char *)glGetString(GL_SHADING_LANGUAGE_VERSION);
    printf("wine-build smoke test: %s %s GL_RENDERER: %s\n", ARCH, what,
           renderer ? renderer : "(null)");
    printf("wine-build smoke test: %s %s GL_VERSION: %s\n", ARCH, what,
           version ? version : "(null)");
    printf("wine-build smoke test: %s %s GL_SHADING_LANGUAGE_VERSION: %s\n", ARCH, what,
           glsl ? glsl : "(null)");
    if (!renderer || !version)
    {
        printf("wine-build smoke test: %s %s has no GL_RENDERER or GL_VERSION\n", ARCH, what);
        return FAIL_STRINGS;
    }
    if (expect_renderer && !strstr(renderer, expect_renderer))
    {
        printf("wine-build smoke test: %s %s renderer is not %s\n", ARCH, what, expect_renderer);
        return FAIL_RENDERER;
    }
    if ((ret = clear_and_check(what, 64, 128, 192))) return ret;
    for (i = 0; i < 5; i++)
    {
        if ((ret = clear_and_check(what, 32 * i, 255 - 32 * i, 96))) return ret;
        if (!SwapBuffers(dc))
        {
            printf("wine-build smoke test: %s %s SwapBuffers failed, error %lu\n", ARCH, what,
                   GetLastError());
            return FAIL_SWAP;
        }
        pump();
    }
    printf("wine-build smoke test: %s %s clears, reads back and swaps\n", ARCH, what);
    return 0;
}

/* Print the average time of 60 swaps of the current context with swap
 * interval <interval>, after 5 swaps to let the interval take effect. */
static int time_swaps(HDC dc, PFN_wglSwapIntervalEXT swap_interval, int interval)
{
    LARGE_INTEGER freq, start, end;
    int i;

    if (!swap_interval(interval))
    {
        printf("wine-build smoke test: %s wglSwapIntervalEXT(%d) failed, error %lu\n", ARCH,
               interval, GetLastError());
        return FAIL_SWAP;
    }
    QueryPerformanceFrequency(&freq);
    for (i = 0; i < 65; i++)
    {
        if (i == 5) QueryPerformanceCounter(&start);
        glClearColor((i & 1) ? 1.0f : 0.0f, 0.0f, 0.0f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT);
        if (!SwapBuffers(dc))
        {
            printf("wine-build smoke test: %s SwapBuffers failed, error %lu\n", ARCH,
                   GetLastError());
            return FAIL_SWAP;
        }
        pump();
    }
    glFinish();
    QueryPerformanceCounter(&end);
    printf("wine-build smoke test: %s swap interval %d: %.2f ms per frame\n", ARCH, interval,
           (end.QuadPart - start.QuadPart) * 1000.0 / freq.QuadPart / 60);
    return 0;
}

int main(int argc, char **argv)
{
    PIXELFORMATDESCRIPTOR pfd = { sizeof(pfd), 1,
        PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER, PFD_TYPE_RGBA, 32,
        0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 24, 8, 0, PFD_MAIN_PLANE, 0, 0, 0, 0 };
    PFN_wglCreateContextAttribsARB create_context_attribs;
    PFN_wglSwapIntervalEXT swap_interval;
    int core[] = { WGL_CONTEXT_MAJOR_VERSION_ARB, 4, WGL_CONTEXT_MINOR_VERSION_ARB, 6,
                   WGL_CONTEXT_PROFILE_MASK_ARB, WGL_CONTEXT_CORE_PROFILE_BIT_ARB, 0 };
    int compat[] = { WGL_CONTEXT_PROFILE_MASK_ARB, WGL_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB, 0 };
    WNDCLASSA wc = {0};
    HGLRC legacy, ctx = NULL;
    char what[32];
    int format, minor, ret;
    RECT rect;
    HWND hwnd;
    HDC dc;

    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc > 1) expect_renderer = argv[1];

    wc.style = CS_OWNDC;
    wc.lpfnWndProc = DefWindowProcA;
    wc.hInstance = GetModuleHandleA(NULL);
    wc.lpszClassName = "wine-build-opengl";
    if (!RegisterClassA(&wc) ||
        !(hwnd = CreateWindowA(wc.lpszClassName, "wine-build OpenGL smoke test",
                               WS_OVERLAPPEDWINDOW | WS_VISIBLE, 100, 100, 320, 240,
                               NULL, NULL, wc.hInstance, NULL)))
    {
        printf("wine-build smoke test: %s cannot create a window, error %lu\n", ARCH,
               GetLastError());
        return FAIL_WINDOW;
    }
    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);
    pump();
    GetClientRect(hwnd, &rect);
    width = rect.right - rect.left;
    height = rect.bottom - rect.top;
    dc = GetDC(hwnd);

    if (!(format = ChoosePixelFormat(dc, &pfd)) || !SetPixelFormat(dc, format, &pfd))
    {
        printf("wine-build smoke test: %s no pixel format (%d), error %lu\n", ARCH, format,
               GetLastError());
        return FAIL_PIXEL_FORMAT;
    }
    DescribePixelFormat(dc, format, sizeof(pfd), &pfd);
    printf("wine-build smoke test: %s pixel format %d: color %u, alpha %u, depth %u, stencil %u\n",
           ARCH, format, pfd.cColorBits, pfd.cAlphaBits, pfd.cDepthBits, pfd.cStencilBits);

    if (!(legacy = wglCreateContext(dc)))
    {
        printf("wine-build smoke test: %s wglCreateContext failed, error %lu\n", ARCH,
               GetLastError());
        return FAIL_LEGACY_CONTEXT;
    }
    if ((ret = exercise("legacy", dc, legacy))) return ret;

    create_context_attribs = (PFN_wglCreateContextAttribsARB)(void *)
        wglGetProcAddress("wglCreateContextAttribsARB");
    if (!create_context_attribs)
    {
        printf("wine-build smoke test: %s has no wglCreateContextAttribsARB\n", ARCH);
        return FAIL_NO_CREATE_CONTEXT_ATTRIBS;
    }

    for (minor = 6; minor >= 0 && !ctx; minor--)
    {
        core[3] = minor;
        ctx = create_context_attribs(dc, NULL, core);
    }
    if (!ctx)
    {
        printf("wine-build smoke test: %s no 4.x core context, error %lu\n", ARCH,
               GetLastError());
        return FAIL_CORE_CONTEXT;
    }
    snprintf(what, sizeof(what), "core 4.%d", minor + 1);
    if ((ret = exercise(what, dc, ctx))) return ret;
    swap_interval = (PFN_wglSwapIntervalEXT)(void *)wglGetProcAddress("wglSwapIntervalEXT");
    if (!swap_interval)
    {
        printf("wine-build smoke test: %s has no wglSwapIntervalEXT\n", ARCH);
        return FAIL_SWAP;
    }
    if ((ret = time_swaps(dc, swap_interval, 1))) return ret;
    if ((ret = time_swaps(dc, swap_interval, 0))) return ret;
    wglMakeCurrent(NULL, NULL);
    wglDeleteContext(ctx);

    if (!(ctx = create_context_attribs(dc, NULL, compat)))
    {
        printf("wine-build smoke test: %s no compatibility context, error %lu\n", ARCH,
               GetLastError());
        return FAIL_COMPAT_CONTEXT;
    }
    if ((ret = exercise("compat", dc, ctx))) return ret;
    wglMakeCurrent(NULL, NULL);
    wglDeleteContext(ctx);
    wglDeleteContext(legacy);

    ReleaseDC(hwnd, dc);
    DestroyWindow(hwnd);
    printf("wine-build smoke test: %s OpenGL contexts work\n", ARCH);
    return 0;
}
