/* The host program bundle-wine.sh builds to check the bundle's EGL outside
 * Wine, on a Mac with a Metal 4 GPU. It opens the libEGL.1.dylib it is given
 * (lib/external of the bundle), initializes EGL's surfaceless platform, the
 * one winemac.drv uses, binds desktop OpenGL, makes a context current without
 * a surface and prints GL_RENDERER and GL_VERSION, which must name Zink on
 * KosmicKrisp. It also prints the path of each EGL, gallium and Vulkan
 * library the process loaded, so the caller can check that they all came from
 * the bundle. The caller points VK_DRIVER_FILES at the bundle's manifest, as
 * ntdll does for Wine. The exit status names the step that failed, after a
 * line saying why.
 *
 * The EGL declarations it needs are written out here, so it builds without
 * EGL headers. */

#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef void *EGLDisplay;
typedef void *EGLConfig;
typedef void *EGLContext;
typedef int32_t EGLint;
typedef unsigned int EGLBoolean;
typedef unsigned int EGLenum;

#define EGL_PLATFORM_SURFACELESS_MESA 0x31DD
#define EGL_OPENGL_API 0x30A2
#define EGL_RENDERABLE_TYPE 0x3040
#define EGL_OPENGL_BIT 0x0008
#define EGL_NONE 0x3038
#define EGL_NO_SURFACE ((void *)0)
#define EGL_NO_CONTEXT ((void *)0)
#define EGL_VENDOR 0x3053
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02

typedef EGLDisplay (*get_platform_display_t)(EGLenum, void *, const intptr_t *);
typedef EGLBoolean (*initialize_t)(EGLDisplay, EGLint *, EGLint *);
typedef const char *(*query_string_t)(EGLDisplay, EGLint);
typedef EGLBoolean (*bind_api_t)(EGLenum);
typedef EGLBoolean (*choose_config_t)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *);
typedef EGLContext (*create_context_t)(EGLDisplay, EGLConfig, EGLContext, const EGLint *);
typedef EGLBoolean (*make_current_t)(EGLDisplay, void *, void *, EGLContext);
typedef EGLint (*get_error_t)(void);
typedef void *(*get_proc_address_t)(const char *);
typedef EGLBoolean (*terminate_t)(EGLDisplay);
typedef const unsigned char *(*get_string_t)(unsigned int);

enum
{
    FAIL_DLOPEN = 1,
    FAIL_SYMBOLS,
    FAIL_INITIALIZE,
    FAIL_CONFIG,
    FAIL_CONTEXT,
    FAIL_STRINGS,
};

int main(int argc, char **argv)
{
    static const EGLint config_attribs[] = { EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE };
    get_platform_display_t get_platform_display;
    initialize_t initialize;
    query_string_t query_string;
    bind_api_t bind_api;
    choose_config_t choose_config;
    create_context_t create_context;
    make_current_t make_current;
    get_error_t get_error;
    get_proc_address_t get_proc_address;
    terminate_t terminate;
    get_string_t get_string;
    const char *renderer, *version;
    EGLint major = 0, minor = 0, count = 0;
    EGLDisplay display;
    EGLContext context;
    EGLConfig config;
    uint32_t i;
    void *egl;

    if (argc != 2)
    {
        printf("usage: %s <libEGL.1.dylib>\n", argv[0]);
        return FAIL_DLOPEN;
    }
    if (!(egl = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("egl: cannot load %s: %s\n", argv[1], dlerror());
        return FAIL_DLOPEN;
    }
    get_platform_display = (get_platform_display_t)dlsym(egl, "eglGetPlatformDisplay");
    initialize = (initialize_t)dlsym(egl, "eglInitialize");
    query_string = (query_string_t)dlsym(egl, "eglQueryString");
    bind_api = (bind_api_t)dlsym(egl, "eglBindAPI");
    choose_config = (choose_config_t)dlsym(egl, "eglChooseConfig");
    create_context = (create_context_t)dlsym(egl, "eglCreateContext");
    make_current = (make_current_t)dlsym(egl, "eglMakeCurrent");
    get_error = (get_error_t)dlsym(egl, "eglGetError");
    get_proc_address = (get_proc_address_t)dlsym(egl, "eglGetProcAddress");
    terminate = (terminate_t)dlsym(egl, "eglTerminate");
    if (!get_platform_display || !initialize || !query_string || !bind_api || !choose_config ||
        !create_context || !make_current || !get_error || !get_proc_address || !terminate)
    {
        printf("egl: %s lacks an EGL 1.5 entry point\n", argv[1]);
        return FAIL_SYMBOLS;
    }

    display = get_platform_display(EGL_PLATFORM_SURFACELESS_MESA, NULL, NULL);
    if (!display || !initialize(display, &major, &minor))
    {
        printf("egl: eglInitialize on the surfaceless platform failed, error %#x\n", get_error());
        return FAIL_INITIALIZE;
    }
    printf("egl: EGL %d.%d, vendor %s\n", major, minor, query_string(display, EGL_VENDOR));

    if (!bind_api(EGL_OPENGL_API) || !choose_config(display, config_attribs, &config, 1, &count) ||
        !count)
    {
        printf("egl: no OpenGL config, error %#x\n", get_error());
        return FAIL_CONFIG;
    }
    if (!(context = create_context(display, config, EGL_NO_CONTEXT, NULL)) ||
        !make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, context))
    {
        printf("egl: cannot make an OpenGL context current, error %#x\n", get_error());
        return FAIL_CONTEXT;
    }
    get_string = (get_string_t)get_proc_address("glGetString");
    renderer = get_string ? (const char *)get_string(GL_RENDERER) : NULL;
    version = get_string ? (const char *)get_string(GL_VERSION) : NULL;
    if (!renderer || !version)
    {
        printf("egl: no GL_RENDERER or GL_VERSION\n");
        return FAIL_STRINGS;
    }
    printf("egl: GL_RENDERER: %s\n", renderer);
    printf("egl: GL_VERSION: %s\n", version);

    for (i = 0; i < _dyld_image_count(); i++)
    {
        const char *name = _dyld_get_image_name(i);
        if (strstr(name, "libEGL") || strstr(name, "libgallium") || strstr(name, "libvulkan"))
            printf("egl: loaded %s\n", name);
    }
    make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    terminate(display);
    return 0;
}
