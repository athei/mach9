/* The host program bundle-wine.sh builds to check that OpenGL texture
 * transfers through pixel buffer objects on Zink on KosmicKrisp copy float
 * texels bit for bit, outside Wine, on a Mac with a Metal 4 GPU. It makes a
 * core context current on the libEGL.1.dylib it is given, as smoke/egl.c
 * does, and moves NaNs with payloads (quiet and signalling, both signs),
 * infinities, denormals and negative zero through textures of the formats
 * GL_R32F, GL_RGBA32F and GL_RGBA16F, -128 and -127 through
 * GL_RGBA8_SNORM, and arbitrary 32-bit words through the packed float
 * formats GL_R11F_G11F_B10F and GL_RGB9_E5, with the client format and
 * type that match the texture: glTexSubImage2D from a
 * GL_PIXEL_UNPACK_BUFFER and from client memory, then glGetTexImage and,
 * for a color-renderable format, glReadPixels (from a framebuffer with the
 * texture attached) into a GL_PIXEL_PACK_BUFFER and into client memory,
 * with glReadPixels' clamping off. wined3d copies typeless Direct3D
 * resources this way. Every transfer has to return the bits it was given.
 * It prints one line per texel that differs and exits with the number of
 * them, at most 100; a status above 100 names a setup step that failed.
 *
 * The EGL and GL declarations it needs are written out here, so it builds
 * without headers. */

#include <dlfcn.h>
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
#define EGL_CONTEXT_MAJOR_VERSION 0x3098
#define EGL_CONTEXT_MINOR_VERSION 0x30FB
#define EGL_CONTEXT_OPENGL_PROFILE_MASK 0x30FD
#define EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT 0x0001

typedef unsigned int GLenum;
typedef unsigned int GLuint;
typedef int GLint;
typedef int GLsizei;
typedef intptr_t GLintptr;
typedef intptr_t GLsizeiptr;

#define GL_TEXTURE_2D 0x0DE1
#define GL_UNPACK_ALIGNMENT 0x0CF5
#define GL_PACK_ALIGNMENT 0x0D05
#define GL_BYTE 0x1400
#define GL_HALF_FLOAT 0x140B
#define GL_RGBA8_SNORM 0x8F97
#define GL_RGB 0x1907
#define GL_R11F_G11F_B10F 0x8C3A
#define GL_UNSIGNED_INT_10F_11F_11F_REV 0x8C3B
#define GL_RGB9_E5 0x8C3D
#define GL_UNSIGNED_INT_5_9_9_9_REV 0x8C3E
#define GL_CLAMP_READ_COLOR 0x891C
#define GL_FLOAT 0x1406
#define GL_RED 0x1903
#define GL_RGBA 0x1908
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02
#define GL_R32F 0x822E
#define GL_RGBA32F 0x8814
#define GL_RGBA16F 0x881A
#define GL_STREAM_DRAW 0x88E0
#define GL_STREAM_READ 0x88E1
#define GL_PIXEL_PACK_BUFFER 0x88EB
#define GL_PIXEL_UNPACK_BUFFER 0x88EC
#define GL_FRAMEBUFFER_COMPLETE 0x8CD5
#define GL_COLOR_ATTACHMENT0 0x8CE0
#define GL_FRAMEBUFFER 0x8D40

#define GL_FUNCS \
    X(void, glGenTextures, (GLsizei, GLuint *)) \
    X(void, glBindTexture, (GLenum, GLuint)) \
    X(void, glTexStorage2D, (GLenum, GLsizei, GLenum, GLsizei, GLsizei)) \
    X(void, glTexSubImage2D, (GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, const void *)) \
    X(void, glGetTexImage, (GLenum, GLint, GLenum, GLenum, void *)) \
    X(void, glPixelStorei, (GLenum, GLint)) \
    X(void, glGenBuffers, (GLsizei, GLuint *)) \
    X(void, glBindBuffer, (GLenum, GLuint)) \
    X(void, glBufferData, (GLenum, GLsizeiptr, const void *, GLenum)) \
    X(void, glGetBufferSubData, (GLenum, GLintptr, GLsizeiptr, void *)) \
    X(void, glGenFramebuffers, (GLsizei, GLuint *)) \
    X(void, glBindFramebuffer, (GLenum, GLuint)) \
    X(void, glFramebufferTexture2D, (GLenum, GLenum, GLenum, GLuint, GLint)) \
    X(GLenum, glCheckFramebufferStatus, (GLenum)) \
    X(void, glReadBuffer, (GLenum)) \
    X(void, glReadPixels, (GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, void *)) \
    X(void, glClampColor, (GLenum, GLenum)) \
    X(GLenum, glGetError, (void)) \
    X(const unsigned char *, glGetString, (GLenum))

#define X(ret, name, args) static ret (*p_##name) args;
GL_FUNCS
#undef X

enum
{
    FAIL_DLOPEN = 101,
    FAIL_INITIALIZE,
    FAIL_CONTEXT,
    FAIL_FUNCTIONS,
};

#define WIDTH 4
#define HEIGHT 2
#define MAX_BYTES (WIDTH * HEIGHT * 16)

/* Bit patterns: NaNs with payloads, quiet and signalling, positive and
 * negative, then infinity, the smallest denormal, negative zero and 1.0. */
static const uint32_t bits32[] =
{
    0x7fc00001, 0x7fd23456, 0x7f800001, 0x7fa00000,
    0xffc12345, 0xff800abc, 0x7f800000, 0x00000001,
    0x80000000, 0x3f800000, 0x7fffffff, 0xffbfffff,
    0x7fc0ffee, 0x7f8f0f0f, 0xfff00001, 0x80000001,
};
static const uint16_t bits16[] =
{
    0x7e01, 0x7c01, 0xfe55, 0x7d00,
    0xfc3f, 0x7c00, 0x0001, 0x8000,
    0x3c00, 0x7fff, 0xfdff, 0x7e80,
    0x7c80, 0xffff, 0x8001, 0x7f01,
};
/* -128 and -127 both read as -1.0 in a signed normalized format. */
static const uint8_t bits8[] =
{
    0x80, 0x81, 0x7f, 0x00, 0xff, 0x01, 0x80, 0x80,
    0x40, 0xc0, 0x80, 0x7f, 0x81, 0x80, 0x00, 0x80,
};

struct format
{
    const char *name;
    GLenum internal, format, type;
    unsigned int texel, channel; /* bytes */
    int renderable;
};

static const struct format formats[] =
{
    { "GL_R32F", GL_R32F, GL_RED, GL_FLOAT, 4, 4, 1 },
    { "GL_RGBA32F", GL_RGBA32F, GL_RGBA, GL_FLOAT, 16, 4, 1 },
    { "GL_RGBA16F", GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, 8, 2, 1 },
    { "GL_RGBA8_SNORM", GL_RGBA8_SNORM, GL_RGBA, GL_BYTE, 4, 1, 1 },
    /* a 32-bit word per texel */
    { "GL_R11F_G11F_B10F", GL_R11F_G11F_B10F, GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV, 4, 4, 1 },
    { "GL_RGB9_E5", GL_RGB9_E5, GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV, 4, 4, 0 },
};

static unsigned int failures;

static void fill(const struct format *f, unsigned char *data)
{
    unsigned int i, n = WIDTH * HEIGHT * f->texel / f->channel;

    for (i = 0; i < n; i++)
    {
        if (f->channel == 4)
            memcpy(data + i * 4, &bits32[i % 16], 4);
        else if (f->channel == 2)
            memcpy(data + i * 2, &bits16[i % 16], 2);
        else
            data[i] = bits8[i % 16];
    }
}

static void compare(const struct format *f, const char *upload, const char *download,
                    const unsigned char *want, const unsigned char *got)
{
    unsigned int i, n = WIDTH * HEIGHT * f->texel / f->channel;

    for (i = 0; i < n; i++)
    {
        uint32_t w = 0, g = 0;

        memcpy(&w, want + i * f->channel, f->channel);
        memcpy(&g, got + i * f->channel, f->channel);
        if (w != g)
        {
            printf("glpbo: %s, upload %s, %s: value %u is %#x, expected %#x\n",
                   f->name, upload, download, i, g, w);
            failures++;
        }
    }
}

static void check_error(const struct format *f, const char *what)
{
    GLenum error = p_glGetError();

    if (error)
    {
        printf("glpbo: %s, %s: GL error %#x\n", f->name, what, error);
        failures++;
    }
}

static void test_format(const struct format *f, int pbo_upload)
{
    const char *upload = pbo_upload ? "from a pixel buffer object" : "from client memory";
    unsigned char want[MAX_BYTES], got[MAX_BYTES];
    unsigned int size = WIDTH * HEIGHT * f->texel;
    GLuint tex, fbo, unpack, pack;

    fill(f, want);
    p_glGenTextures(1, &tex);
    p_glBindTexture(GL_TEXTURE_2D, tex);
    p_glTexStorage2D(GL_TEXTURE_2D, 1, f->internal, WIDTH, HEIGHT);
    if (pbo_upload)
    {
        p_glGenBuffers(1, &unpack);
        p_glBindBuffer(GL_PIXEL_UNPACK_BUFFER, unpack);
        p_glBufferData(GL_PIXEL_UNPACK_BUFFER, size, want, GL_STREAM_DRAW);
        p_glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, WIDTH, HEIGHT, f->format, f->type, NULL);
        p_glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
    }
    else
    {
        p_glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, WIDTH, HEIGHT, f->format, f->type, want);
    }
    check_error(f, upload);

    memset(got, 0x55, sizeof(got));
    p_glGetTexImage(GL_TEXTURE_2D, 0, f->format, f->type, got);
    compare(f, upload, "glGetTexImage into client memory", want, got);

    p_glGenBuffers(1, &pack);
    p_glBindBuffer(GL_PIXEL_PACK_BUFFER, pack);
    p_glBufferData(GL_PIXEL_PACK_BUFFER, size, NULL, GL_STREAM_READ);
    p_glGetTexImage(GL_TEXTURE_2D, 0, f->format, f->type, NULL);
    memset(got, 0x55, sizeof(got));
    p_glGetBufferSubData(GL_PIXEL_PACK_BUFFER, 0, size, got);
    p_glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
    compare(f, upload, "glGetTexImage into a pixel buffer object", want, got);
    check_error(f, "glGetTexImage");
    if (!f->renderable)
        return;

    p_glGenFramebuffers(1, &fbo);
    p_glBindFramebuffer(GL_FRAMEBUFFER, fbo);
    p_glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    if (p_glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    {
        printf("glpbo: %s: the framebuffer is not complete\n", f->name);
        failures++;
        return;
    }
    p_glReadBuffer(GL_COLOR_ATTACHMENT0);

    memset(got, 0x55, sizeof(got));
    p_glReadPixels(0, 0, WIDTH, HEIGHT, f->format, f->type, got);
    compare(f, upload, "glReadPixels into client memory", want, got);

    p_glBindBuffer(GL_PIXEL_PACK_BUFFER, pack);
    p_glBufferData(GL_PIXEL_PACK_BUFFER, size, NULL, GL_STREAM_READ);
    p_glReadPixels(0, 0, WIDTH, HEIGHT, f->format, f->type, NULL);
    memset(got, 0x55, sizeof(got));
    p_glGetBufferSubData(GL_PIXEL_PACK_BUFFER, 0, size, got);
    p_glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
    compare(f, upload, "glReadPixels into a pixel buffer object", want, got);
    p_glBindFramebuffer(GL_FRAMEBUFFER, 0);
    check_error(f, "glReadPixels");
}

int main(int argc, char **argv)
{
    static const EGLint config_attribs[] = { EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE };
    static const EGLint context_attribs[] =
    {
        EGL_CONTEXT_MAJOR_VERSION, 4, EGL_CONTEXT_MINOR_VERSION, 6,
        EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT, EGL_NONE,
    };
    EGLDisplay (*get_platform_display)(EGLenum, void *, const intptr_t *);
    EGLBoolean (*initialize)(EGLDisplay, EGLint *, EGLint *);
    EGLBoolean (*bind_api)(EGLenum);
    EGLBoolean (*choose_config)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *);
    EGLContext (*create_context)(EGLDisplay, EGLConfig, EGLContext, const EGLint *);
    EGLBoolean (*make_current)(EGLDisplay, void *, void *, EGLContext);
    void *(*get_proc_address)(const char *);
    EGLBoolean (*terminate)(EGLDisplay);
    EGLint major = 0, minor = 0, count = 0;
    EGLDisplay display;
    EGLContext context;
    EGLConfig config;
    unsigned int i;
    void *egl;

    if (argc != 2)
    {
        printf("usage: %s <libEGL.1.dylib>\n", argv[0]);
        return FAIL_DLOPEN;
    }
    if (!(egl = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("glpbo: cannot load %s: %s\n", argv[1], dlerror());
        return FAIL_DLOPEN;
    }
    get_platform_display = dlsym(egl, "eglGetPlatformDisplay");
    initialize = dlsym(egl, "eglInitialize");
    bind_api = dlsym(egl, "eglBindAPI");
    choose_config = dlsym(egl, "eglChooseConfig");
    create_context = dlsym(egl, "eglCreateContext");
    make_current = dlsym(egl, "eglMakeCurrent");
    get_proc_address = dlsym(egl, "eglGetProcAddress");
    terminate = dlsym(egl, "eglTerminate");
    if (!get_platform_display || !initialize || !bind_api || !choose_config || !create_context ||
        !make_current || !get_proc_address || !terminate)
    {
        printf("glpbo: %s lacks an EGL 1.5 entry point\n", argv[1]);
        return FAIL_DLOPEN;
    }
    display = get_platform_display(EGL_PLATFORM_SURFACELESS_MESA, NULL, NULL);
    if (!display || !initialize(display, &major, &minor))
    {
        printf("glpbo: eglInitialize on the surfaceless platform failed\n");
        return FAIL_INITIALIZE;
    }
    if (!bind_api(EGL_OPENGL_API) || !choose_config(display, config_attribs, &config, 1, &count) ||
        !count || !(context = create_context(display, config, EGL_NO_CONTEXT, context_attribs)) ||
        !make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, context))
    {
        printf("glpbo: cannot make an OpenGL 4.6 core context current\n");
        return FAIL_CONTEXT;
    }

#define X(ret, name, args) \
    if (!(p_##name = (ret (*) args)get_proc_address(#name))) \
    { \
        printf("glpbo: no %s\n", #name); \
        return FAIL_FUNCTIONS; \
    }
    GL_FUNCS
#undef X
    printf("glpbo: GL_RENDERER: %s\n", p_glGetString(GL_RENDERER));
    printf("glpbo: GL_VERSION: %s\n", p_glGetString(GL_VERSION));
    p_glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
    p_glPixelStorei(GL_PACK_ALIGNMENT, 1);
    /* Otherwise glReadPixels clamps signed normalized values to [0, 1]. */
    p_glClampColor(GL_CLAMP_READ_COLOR, 0);

    for (i = 0; i < sizeof(formats) / sizeof(formats[0]); i++)
    {
        test_format(&formats[i], 0);
        test_format(&formats[i], 1);
    }

    printf("glpbo: %u wrong values\n", failures);
    make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    terminate(display);
    return failures > 100 ? 100 : failures;
}
