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
 * With the argument depth-bias it checks glPolygonOffset on 24-bit depth
 * buffers instead. KosmicKrisp has no D24 format, so Zink stores them as
 * 32-bit float, on which the device's constant bias would depend on the
 * depth; Zink applies it in the fragment shader with the 24-bit step. Each
 * case draws a quad into a 64x64 framebuffer once without and once with the
 * offset and prints the difference at the centre pixel in units of 2^-24;
 * one unit of glPolygonOffset is two of them, as on Mesa's drivers with a
 * real D24 format. Cases: first a quad drawn as GL_QUADS, which Zink draws
 * through a geometry shader of its own, so that after it and the
 * glReadPixels blit the next draw of another primitive has to unbind that
 * shader; a flat quad at
 * several depths; a quad with a slope of 256 units per pixel (slope alone,
 * which the device's bias applies, and with a constant); a slope of 100.37
 * units per pixel at z 0.75 with factors 4 and 16, where the shader's slope
 * from float derivatives is off by up to the factor; the sloped quad drawn as
 * lines with GL_POLYGON_OFFSET_LINE, which keeps the device's bias; shaders
 * that read gl_FragCoord.z or only gl_FragCoord.xy; a quad at the far
 * plane, which a 24-bit buffer saturates to 1.0; and quads drawn over
 * themselves that pass the depth test only with the offset (draw_over): a
 * negative constant under GL_LESS, a positive one under GL_GREATER, each
 * alone and with a factor of the other sign, flat and on the slope of 256
 * units per pixel, where the factor decides the sign; one as GL_QUADS; and
 * with GL_DEPTH_CLAMP a quad beyond the far plane. Then, with 4 samples, the
 * difference at each sample of the centre pixel, which a shader reads from
 * the multisampled depth texture: a flat quad at several depths, and the
 * quad with the slope of 256 units per pixel, whose samples differ in depth,
 * also with a slope factor; with a shader that reads gl_FragCoord.z, whose
 * red has to equal the sample's depth, sloped in x, in y and in both; and the
 * quad sloped in x with its right edge through the centre pixel left of the
 * centre, whose covered samples have to get the offset too.
 *
 * The EGL and GL declarations it needs are written out here, so it builds
 * without EGL or GL headers. */

#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <math.h>
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
    FAIL_DEPTH_BIAS,
};

#define GL_DEPTH_BUFFER_BIT 0x0100
#define GL_COLOR_BUFFER_BIT 0x4000
#define GL_TRIANGLE_STRIP 0x0005
#define GL_QUADS 0x0007
#define GL_LESS 0x0201
#define GL_LEQUAL 0x0203
#define GL_GREATER 0x0204
#define GL_ALWAYS 0x0207
#define GL_DEPTH_CLAMP 0x864F
#define GL_DEPTH_TEST 0x0B71
#define GL_POLYGON_OFFSET_FILL 0x8037
#define GL_POLYGON_OFFSET_LINE 0x2A02
#define GL_FRONT_AND_BACK 0x0408
#define GL_LINE 0x1B01
#define GL_FILL 0x1B02
#define GL_RED 0x1903
#define GL_FLOAT 0x1406
#define GL_DEPTH_COMPONENT 0x1902
#define GL_DEPTH_COMPONENT24 0x81A6
#define GL_DEPTH24_STENCIL8 0x88F0
#define GL_R32F 0x822E
#define GL_RGBA 0x1908
#define GL_RGBA32F 0x8814
#define GL_TEXTURE_2D_MULTISAMPLE 0x9100
#define GL_FRAMEBUFFER 0x8D40
#define GL_RENDERBUFFER 0x8D41
#define GL_COLOR_ATTACHMENT0 0x8CE0
#define GL_DEPTH_ATTACHMENT 0x8D00
#define GL_DEPTH_STENCIL_ATTACHMENT 0x821A
#define GL_FRAMEBUFFER_COMPLETE 0x8CD5
#define GL_FRAGMENT_SHADER 0x8B30
#define GL_VERTEX_SHADER 0x8B31
#define GL_COMPILE_STATUS 0x8B81
#define GL_LINK_STATUS 0x8B82

/* the GL entry points the depth-bias cases use, in the order of gl_names */
static struct
{
    void (*Viewport)(int, int, int, int);
    void (*Enable)(unsigned int);
    void (*Disable)(unsigned int);
    void (*DepthFunc)(unsigned int);
    void (*ClearDepth)(double);
    void (*ClearColor)(float, float, float, float);
    void (*Clear)(unsigned int);
    void (*PolygonOffset)(float, float);
    void (*Begin)(unsigned int);
    void (*End)(void);
    void (*Color4f)(float, float, float, float);
    void (*Vertex3f)(float, float, float);
    void (*ReadPixels)(int, int, int, int, unsigned int, unsigned int, void *);
    unsigned int (*GetError)(void);
    void (*GenFramebuffers)(int, unsigned int *);
    void (*BindFramebuffer)(unsigned int, unsigned int);
    void (*FramebufferRenderbuffer)(unsigned int, unsigned int, unsigned int, unsigned int);
    unsigned int (*CheckFramebufferStatus)(unsigned int);
    void (*GenRenderbuffers)(int, unsigned int *);
    void (*BindRenderbuffer)(unsigned int, unsigned int);
    void (*RenderbufferStorage)(unsigned int, unsigned int, int, int);
    unsigned int (*CreateShader)(unsigned int);
    void (*ShaderSource)(unsigned int, int, const char *const *, const int *);
    void (*CompileShader)(unsigned int);
    void (*GetShaderiv)(unsigned int, unsigned int, int *);
    unsigned int (*CreateProgram)(void);
    void (*AttachShader)(unsigned int, unsigned int);
    void (*LinkProgram)(unsigned int);
    void (*GetProgramiv)(unsigned int, unsigned int, int *);
    void (*UseProgram)(unsigned int);
    void (*PolygonMode)(unsigned int, unsigned int);
    void (*GenTextures)(int, unsigned int *);
    void (*BindTexture)(unsigned int, unsigned int);
    void (*TexImage2DMultisample)(unsigned int, int, unsigned int, int, int, unsigned char);
    void (*FramebufferTexture2D)(unsigned int, unsigned int, unsigned int, unsigned int, int);
} gl;

static const char *const gl_names[] = {
    "glViewport", "glEnable", "glDisable", "glDepthFunc", "glClearDepth", "glClearColor", "glClear",
    "glPolygonOffset", "glBegin", "glEnd", "glColor4f", "glVertex3f", "glReadPixels", "glGetError",
    "glGenFramebuffers", "glBindFramebuffer", "glFramebufferRenderbuffer",
    "glCheckFramebufferStatus", "glGenRenderbuffers", "glBindRenderbuffer", "glRenderbufferStorage",
    "glCreateShader", "glShaderSource", "glCompileShader", "glGetShaderiv", "glCreateProgram",
    "glAttachShader", "glLinkProgram", "glGetProgramiv", "glUseProgram", "glPolygonMode",
    "glGenTextures", "glBindTexture", "glTexImage2DMultisample", "glFramebufferTexture2D",
};

#define SIZE 64
#define UNIT (1.0 / (1 << 24))
#define SAMPLES 4

/* the fragment shaders of the gl_FragCoord cases; the vertex shader passes the
 * vertex through */
static const char vs_source[] = "#version 120\nvoid main() { gl_Position = gl_Vertex; }\n";
static const char fs_z_source[] =
    "#version 120\nvoid main() { gl_FragColor = vec4(gl_FragCoord.z, 0.0, 0.0, 1.0); }\n";
static const char fs_xy_source[] =
    "#version 120\nvoid main() { gl_FragColor = vec4(gl_FragCoord.x + gl_FragCoord.y, 0.0, 0.0, 1.0); }\n";
/* reads samples 0 to 3 of a multisampled texture's red into red, green, blue
 * and alpha */
static const char vs_fetch_source[] = "#version 130\nvoid main() { gl_Position = gl_Vertex; }\n";
static const char fs_fetch_source[] =
    "#version 130\n#extension GL_ARB_texture_multisample : require\nuniform sampler2DMS tex;\n"
    "void main() { ivec2 p = ivec2(gl_FragCoord.xy); gl_FragColor = vec4(texelFetch(tex, p, 0).r, "
    "texelFetch(tex, p, 1).r, texelFetch(tex, p, 2).r, texelFetch(tex, p, 3).r); }\n";

static unsigned int make_program(const char *vs, const char *fs_source)
{
    unsigned int program = gl.CreateProgram(), shader;
    const char *sources[2] = { vs, fs_source };
    unsigned int types[2] = { GL_VERTEX_SHADER, GL_FRAGMENT_SHADER };
    int i, ok;

    for (i = 0; i < 2; i++)
    {
        shader = gl.CreateShader(types[i]);
        gl.ShaderSource(shader, 1, &sources[i], NULL);
        gl.CompileShader(shader);
        gl.GetShaderiv(shader, GL_COMPILE_STATUS, &ok);
        if (!ok)
            return 0;
        gl.AttachShader(program, shader);
    }
    gl.LinkProgram(program);
    gl.GetProgramiv(program, GL_LINK_STATUS, &ok);
    return ok ? program : 0;
}

/* GL_TRIANGLE_STRIP, or GL_QUADS, which Zink draws through a geometry
 * shader it generates */
static unsigned int quad_mode = GL_TRIANGLE_STRIP;

/* a quad over the framebuffer from window depth left to right */
static void draw_quad(double left, double right)
{
    float l = (float)(2.0 * left - 1.0), r = (float)(2.0 * right - 1.0);

    gl.Begin(quad_mode);
    gl.Vertex3f(-1.0f, -1.0f, l);
    gl.Vertex3f(1.0f, -1.0f, r);
    if (quad_mode == GL_QUADS)
    {
        gl.Vertex3f(1.0f, 1.0f, r);
        gl.Vertex3f(-1.0f, 1.0f, l);
    }
    else
    {
        gl.Vertex3f(-1.0f, 1.0f, l);
        gl.Vertex3f(1.0f, 1.0f, r);
    }
    gl.End();
}

/* draw the quad without and with the offset, each into a cleared framebuffer
 * with the depth test func, and read the centre pixel's depth and red */
static void draw_twice(double left, double right, float factor, float units, unsigned int func,
                       float depth[2], float red[2])
{
    int i;

    gl.DepthFunc(func);
    for (i = 0; i < 2; i++)
    {
        gl.ClearDepth(1.0);
        gl.ClearColor(0.0f, 0.0f, 0.0f, 0.0f);
        gl.Clear(GL_DEPTH_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
        if (i)
        {
            gl.Enable(GL_POLYGON_OFFSET_FILL);
            gl.PolygonOffset(factor, units);
        }
        draw_quad(left, right);
        gl.Disable(GL_POLYGON_OFFSET_FILL);
        gl.ReadPixels(SIZE / 2, SIZE / 2, 1, 1, GL_DEPTH_COMPONENT, GL_FLOAT, &depth[i]);
        gl.ReadPixels(SIZE / 2, SIZE / 2, 1, 1, GL_RED, GL_FLOAT, &red[i]);
    }
}

/* a black quad from window depth left to right, then a red one at the same
 * depth with the offset and the depth test func, which passes only on the
 * offset depth; returns the centre pixel's red, 1 when the red quad was
 * drawn. Zink declares the shader's depth greater or less than the
 * interpolated one when the bias has one sign, and the device may then test
 * the interpolated depth before the shader; a declaration in the wrong
 * direction rejects the red quad there. */
static float draw_over(double left, double right, float factor, float units, unsigned int func)
{
    float red;

    gl.ClearDepth(1.0);
    gl.ClearColor(0.0f, 0.0f, 0.0f, 0.0f);
    gl.Clear(GL_DEPTH_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
    gl.DepthFunc(GL_ALWAYS);
    gl.Color4f(0.0f, 0.0f, 0.0f, 1.0f);
    draw_quad(left, right);
    gl.DepthFunc(func);
    gl.Color4f(1.0f, 0.0f, 0.0f, 1.0f);
    gl.Enable(GL_POLYGON_OFFSET_FILL);
    gl.PolygonOffset(factor, units);
    draw_quad(left, right);
    gl.Disable(GL_POLYGON_OFFSET_FILL);
    gl.ReadPixels(SIZE / 2, SIZE / 2, 1, 1, GL_RED, GL_FLOAT, &red);
    return red;
}

static int report(const char *format, const char *what, double got, double want, double tolerance)
{
    int ok = fabs(got - want) <= tolerance;
    printf("egl: depth-bias: %s: %s: %.2f, expected %.2f (+-%.2f) %s\n", format, what, got, want,
           tolerance, ok ? "ok" : "FAILED");
    return ok;
}

/* a quad drawn as lines over pixels 16 to 47, without and with
 * GL_POLYGON_OFFSET_LINE, 256 units per pixel from z 0.5 on; returns whether
 * every pixel of the centre row it covers got the offset */
static int check_wireframe(const char *name)
{
    const float l = 0.0f, r = (float)(2.0 * (0.5 + 256.0 * UNIT * SIZE / 2) - 1.0);
    float depth[2][SIZE], red[2][SIZE];
    char what[96];
    int i, x, ok = 1, covered = 0;

    gl.DepthFunc(GL_ALWAYS);
    for (i = 0; i < 2; i++)
    {
        gl.ClearDepth(1.0);
        gl.Clear(GL_DEPTH_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
        gl.PolygonMode(GL_FRONT_AND_BACK, GL_LINE);
        if (i)
        {
            gl.Enable(GL_POLYGON_OFFSET_LINE);
            gl.PolygonOffset(1.0f, 10.0f);
        }
        gl.Begin(GL_TRIANGLE_STRIP);
        gl.Vertex3f(-0.5f, -0.5f, l);
        gl.Vertex3f(0.5f, -0.5f, r);
        gl.Vertex3f(-0.5f, 0.5f, l);
        gl.Vertex3f(0.5f, 0.5f, r);
        gl.End();
        gl.Disable(GL_POLYGON_OFFSET_LINE);
        gl.PolygonMode(GL_FRONT_AND_BACK, GL_FILL);
        gl.ReadPixels(0, SIZE / 2, SIZE, 1, GL_DEPTH_COMPONENT, GL_FLOAT, depth[i]);
        gl.ReadPixels(0, SIZE / 2, SIZE, 1, GL_RED, GL_FLOAT, red[i]);
    }
    for (x = 0; x < SIZE; x++)
    {
        if (red[0][x] != 1.0f || red[1][x] != 1.0f)
            continue;
        covered++;
        snprintf(what, sizeof(what), "GL_LINE, slope 256, factor 1, units 10, pixel %d", x);
        ok &= report(name, what, (depth[1][x] - depth[0][x]) / UNIT, 276.0, 2.0);
    }
    if (!covered)
        printf("egl: depth-bias: %s: GL_LINE: no pixel of the centre row covered FAILED\n", name);
    return ok && covered;
}

/* the cases on one depth format; returns whether all passed */
static int check_depth_format(const char *name, unsigned int format, unsigned int attachment,
                              unsigned int program_z, unsigned int program_xy)
{
    static const double depths[] = { 0.125, 0.25, 0.5, 0.75 };
    static const float factors[] = { 4.0f, 16.0f };
    /* 256 units per pixel across the framebuffer */
    const double slope = 256.0 * UNIT, left = 0.25, right = left + slope * SIZE;
    /* a slope that is not a whole number of units, around z 0.75 */
    const double fine = 100.37 * UNIT, fine_left = 0.75 - fine * SIZE / 2, fine_right = 0.75 + fine * SIZE / 2;
    unsigned int fbo, rb[2];
    float depth[2], red[2];
    char what[96];
    int ok = 1;
    size_t i;

    gl.GenFramebuffers(1, &fbo);
    gl.BindFramebuffer(GL_FRAMEBUFFER, fbo);
    gl.GenRenderbuffers(2, rb);
    gl.BindRenderbuffer(GL_RENDERBUFFER, rb[0]);
    gl.RenderbufferStorage(GL_RENDERBUFFER, GL_R32F, SIZE, SIZE);
    gl.FramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb[0]);
    gl.BindRenderbuffer(GL_RENDERBUFFER, rb[1]);
    gl.RenderbufferStorage(GL_RENDERBUFFER, format, SIZE, SIZE);
    gl.FramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb[1]);
    if (gl.CheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    {
        printf("egl: depth-bias: %s: framebuffer incomplete\n", name);
        return 0;
    }
    gl.Viewport(0, 0, SIZE, SIZE);
    gl.Enable(GL_DEPTH_TEST);
    gl.Color4f(1.0f, 0.0f, 0.0f, 1.0f);

    /* first: after a GL_QUADS draw and a blit, which glReadPixels does here,
     * the next draw of another primitive has to unbind Zink's generated
     * geometry shader */
    quad_mode = GL_QUADS;
    draw_twice(0.25, 0.25, 0.0f, 100.0f, GL_ALWAYS, depth, red);
    ok &= report(name, "GL_QUADS, z 0.250, units 100", (depth[1] - depth[0]) / UNIT, 200.0, 1.0);
    quad_mode = GL_TRIANGLE_STRIP;

    for (i = 0; i < sizeof(depths) / sizeof(depths[0]); i++)
    {
        draw_twice(depths[i], depths[i], 0.0f, 100.0f, GL_ALWAYS, depth, red);
        snprintf(what, sizeof(what), "z %.3f, units 100", depths[i]);
        ok &= report(name, what, (depth[1] - depth[0]) / UNIT, 200.0, 1.0);
        draw_twice(depths[i], depths[i], 0.0f, -100.0f, GL_ALWAYS, depth, red);
        snprintf(what, sizeof(what), "z %.3f, units -100", depths[i]);
        ok &= report(name, what, (depth[1] - depth[0]) / UNIT, -200.0, 1.0);
    }

    draw_twice(left, right, 1.0f, 0.0f, GL_ALWAYS, depth, red);
    ok &= report(name, "slope 256, factor 1", (depth[1] - depth[0]) / UNIT, 256.0, 2.0);
    draw_twice(left, right, 1.0f, 10.0f, GL_ALWAYS, depth, red);
    ok &= report(name, "slope 256, factor 1, units 10", (depth[1] - depth[0]) / UNIT, 276.0, 2.0);
    /* the shader's slope comes from float derivatives of z, which are whole
     * units from z 0.5 on: off by up to the factor */
    for (i = 0; i < sizeof(factors) / sizeof(factors[0]); i++)
    {
        draw_twice(fine_left, fine_right, factors[i], 10.0f, GL_ALWAYS, depth, red);
        snprintf(what, sizeof(what), "z 0.75, slope 100.37, factor %g, units 10", factors[i]);
        ok &= report(name, what, (depth[1] - depth[0]) / UNIT, 100.37 * factors[i] + 20.0, factors[i] + 1.0);
    }
    ok &= check_wireframe(name);

    /* a 24-bit buffer saturates at 1.0, so the biased quad passes GL_LEQUAL
     * against the cleared 1.0 and writes its colour */
    draw_twice(1.0, 1.0, 0.0f, 100.0f, GL_LEQUAL, depth, red);
    ok &= report(name, "z 1.0, units 100, GL_LEQUAL, red", red[1], 1.0, 0.0);

    ok &= report(name, "over itself, units -100, GL_LESS, red",
                 draw_over(0.5, 0.5, 0.0f, -100.0f, GL_LESS), 1.0, 0.0);
    ok &= report(name, "over itself, units 100, GL_GREATER, red",
                 draw_over(0.5, 0.5, 0.0f, 100.0f, GL_GREATER), 1.0, 0.0);
    ok &= report(name, "over itself, factor -1, units 100, GL_GREATER, red",
                 draw_over(0.5, 0.5, -1.0f, 100.0f, GL_GREATER), 1.0, 0.0);
    ok &= report(name, "over itself, factor 1, units -100, GL_LESS, red",
                 draw_over(0.5, 0.5, 1.0f, -100.0f, GL_LESS), 1.0, 0.0);
    /* on the slope of 256 units per pixel the factor outweighs the constant:
     * -4 * 256 + 200 and 4 * 256 - 200 units */
    ok &= report(name, "over itself, slope 256, factor -4, units 100, GL_LESS, red",
                 draw_over(left, right, -4.0f, 100.0f, GL_LESS), 1.0, 0.0);
    ok &= report(name, "over itself, slope 256, factor 4, units -100, GL_GREATER, red",
                 draw_over(left, right, 4.0f, -100.0f, GL_GREATER), 1.0, 0.0);
    /* through Zink's geometry shader, whose program takes the full keys */
    quad_mode = GL_QUADS;
    ok &= report(name, "GL_QUADS, over itself, units -100, GL_LESS, red",
                 draw_over(0.5, 0.5, 0.0f, -100.0f, GL_LESS), 1.0, 0.0);
    quad_mode = GL_TRIANGLE_STRIP;
    /* with GL_DEPTH_CLAMP the quad's interpolated depth is above 1.0 and its
     * saturated one 1.0 */
    gl.Enable(GL_DEPTH_CLAMP);
    ok &= report(name, "GL_DEPTH_CLAMP, z 1.25 over itself, units 100, GL_LEQUAL, red",
                 draw_over(1.25, 1.25, 0.0f, 100.0f, GL_LEQUAL), 1.0, 0.0);
    gl.Disable(GL_DEPTH_CLAMP);

    gl.UseProgram(program_z);
    draw_twice(0.25, 0.25, 0.0f, 100.0f, GL_ALWAYS, depth, red);
    ok &= report(name, "gl_FragCoord.z, units 100, depth", (depth[1] - depth[0]) / UNIT, 200.0, 1.0);
    ok &= report(name, "gl_FragCoord.z, units 100, red minus depth", (red[1] - depth[1]) / UNIT, 0.0, 1.0);
    gl.UseProgram(program_xy);
    draw_twice(0.25, 0.25, 0.0f, 100.0f, GL_ALWAYS, depth, red);
    ok &= report(name, "gl_FragCoord.xy, units 100, depth", (depth[1] - depth[0]) / UNIT, 200.0, 1.0);
    ok &= report(name, "gl_FragCoord.xy, units 100, red", red[1], SIZE + 1.0, 0.0);
    gl.UseProgram(0);
    gl.Disable(GL_DEPTH_TEST);
    gl.BindFramebuffer(GL_FRAMEBUFFER, 0);
    return ok;
}

/* the centre pixel's samples of a multisampled texture, read with
 * program_fetch into the single-sampled framebuffer fetch_fbo */
static void read_samples(unsigned int texture, unsigned int fetch_fbo, unsigned int program_fetch,
                         float samples[SAMPLES])
{
    gl.BindFramebuffer(GL_FRAMEBUFFER, fetch_fbo);
    gl.Disable(GL_DEPTH_TEST);
    gl.UseProgram(program_fetch);
    gl.BindTexture(GL_TEXTURE_2D_MULTISAMPLE, texture);
    draw_quad(0.5, 0.5);
    gl.BindTexture(GL_TEXTURE_2D_MULTISAMPLE, 0);
    gl.UseProgram(0);
    gl.ReadPixels(SIZE / 2, SIZE / 2, 1, 1, GL_RGBA, GL_FLOAT, samples);
}

/* a quad over the framebuffer from its left edge to the x of right, in
 * pixels, with the window depth z0 + dzdx * x + dzdy * y at pixel position
 * (x, y) */
struct plane
{
    double right, z0, dzdx, dzdy;
};

static void draw_plane(const struct plane *p)
{
    static const double corners[4][2] = { { 0, 0 }, { 1, 0 }, { 0, 1 }, { 1, 1 } };
    int i;

    gl.Begin(GL_TRIANGLE_STRIP);
    for (i = 0; i < 4; i++)
    {
        double x = corners[i][0] * p->right, y = corners[i][1] * SIZE;
        double z = p->z0 + p->dzdx * x + p->dzdy * y;
        gl.Vertex3f((float)(2.0 * x / SIZE - 1.0), (float)(2.0 * y / SIZE - 1.0), (float)(2.0 * z - 1.0));
    }
    gl.End();
}

/* draw_twice on the multisampled framebuffer ms_fbo with program, reading
 * the centre pixel's samples of its depth and colour textures */
static void draw_twice_ms(unsigned int ms_fbo, const unsigned int textures[2], unsigned int fetch_fbo,
                          unsigned int program_fetch, unsigned int program, const struct plane *plane,
                          float factor, float units, float depth[2][SAMPLES], float red[2][SAMPLES])
{
    int i;

    for (i = 0; i < 2; i++)
    {
        gl.BindFramebuffer(GL_FRAMEBUFFER, ms_fbo);
        gl.Enable(GL_DEPTH_TEST);
        gl.DepthFunc(GL_ALWAYS);
        gl.ClearDepth(1.0);
        gl.ClearColor(0.0f, 0.0f, 0.0f, 0.0f);
        gl.Clear(GL_DEPTH_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
        if (i)
        {
            gl.Enable(GL_POLYGON_OFFSET_FILL);
            gl.PolygonOffset(factor, units);
        }
        gl.UseProgram(program);
        draw_plane(plane);
        gl.UseProgram(0);
        gl.Disable(GL_POLYGON_OFFSET_FILL);
        read_samples(textures[0], fetch_fbo, program_fetch, depth[i]);
        read_samples(textures[1], fetch_fbo, program_fetch, red[i]);
    }
}

/* the cases on one depth format with 4 samples; every sample has to get the
 * offset of its own depth. A sloped quad's samples differ by up to 192 units
 * at 256 units per pixel, so an offset depth computed at the pixel centre
 * misses at every sample. Returns whether all passed */
static int check_depth_format_ms(const char *name, unsigned int format, unsigned int attachment,
                                 unsigned int program_z, unsigned int program_fetch)
{
    static const double depths[] = { 0.125, 0.25, 0.5, 0.75 };
    static const float units[] = { 100.0f, -100.0f };
    const double slope = 256.0 * UNIT;
    const struct plane in_x = { SIZE, 0.25, slope, 0.0 };
    /* the sloped cases that read gl_FragCoord.z: in x, in y and diagonal */
    const struct plane sloped[] = { in_x, { SIZE, 0.25, 0.0, slope }, { SIZE, 0.25, slope, slope } };
    static const char *const sloped_names[] = { "slope 256 in x", "slope 256 in y", "slope 256 in x and y" };
    /* the right edge through the centre pixel left of its centre, which
     * leaves the centre uncovered and the samples left of it covered */
    const struct plane edge = { SIZE / 2 + 0.45, 0.25, slope, 0.0 };
    unsigned int fbo[2], textures[2], rb;
    float depth[2][SAMPLES], red[2][SAMPLES];
    char what[96];
    int ok = 1, s, covered;
    size_t i, j;

    gl.GenTextures(2, textures);
    gl.BindTexture(GL_TEXTURE_2D_MULTISAMPLE, textures[0]);
    gl.TexImage2DMultisample(GL_TEXTURE_2D_MULTISAMPLE, SAMPLES, format, SIZE, SIZE, 1);
    gl.BindTexture(GL_TEXTURE_2D_MULTISAMPLE, textures[1]);
    gl.TexImage2DMultisample(GL_TEXTURE_2D_MULTISAMPLE, SAMPLES, GL_R32F, SIZE, SIZE, 1);
    gl.BindTexture(GL_TEXTURE_2D_MULTISAMPLE, 0);
    gl.GenFramebuffers(2, fbo);
    gl.BindFramebuffer(GL_FRAMEBUFFER, fbo[0]);
    gl.FramebufferTexture2D(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D_MULTISAMPLE, textures[0], 0);
    gl.FramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D_MULTISAMPLE, textures[1], 0);
    if (gl.CheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    {
        printf("egl: depth-bias: %s: multisampled framebuffer incomplete\n", name);
        return 0;
    }
    gl.GenRenderbuffers(1, &rb);
    gl.BindRenderbuffer(GL_RENDERBUFFER, rb);
    gl.RenderbufferStorage(GL_RENDERBUFFER, GL_RGBA32F, SIZE, SIZE);
    gl.BindFramebuffer(GL_FRAMEBUFFER, fbo[1]);
    gl.FramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    if (gl.CheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    {
        printf("egl: depth-bias: %s: fetch framebuffer incomplete\n", name);
        return 0;
    }
    gl.Viewport(0, 0, SIZE, SIZE);
    gl.Color4f(1.0f, 0.0f, 0.0f, 1.0f);

    for (i = 0; i < sizeof(depths) / sizeof(depths[0]); i++)
    {
        const struct plane flat = { SIZE, depths[i], 0.0, 0.0 };

        for (j = 0; j < sizeof(units) / sizeof(units[0]); j++)
        {
            draw_twice_ms(fbo[0], textures, fbo[1], program_fetch, 0, &flat, 0.0f, units[j], depth, red);
            for (s = 0; s < SAMPLES; s++)
            {
                snprintf(what, sizeof(what), "z %.3f, units %g, sample %d", depths[i], units[j], s);
                ok &= report(name, what, (depth[1][s] - depth[0][s]) / UNIT, 2.0 * units[j], 1.0);
            }
        }
    }

    draw_twice_ms(fbo[0], textures, fbo[1], program_fetch, 0, &in_x, 0.0f, 100.0f, depth, red);
    for (s = 0; s < SAMPLES; s++)
    {
        snprintf(what, sizeof(what), "slope 256 in x, units 100, sample %d", s);
        ok &= report(name, what, (depth[1][s] - depth[0][s]) / UNIT, 200.0, 2.0);
    }
    draw_twice_ms(fbo[0], textures, fbo[1], program_fetch, 0, &in_x, 1.0f, 10.0f, depth, red);
    for (s = 0; s < SAMPLES; s++)
    {
        snprintf(what, sizeof(what), "slope 256 in x, factor 1, units 10, sample %d", s);
        ok &= report(name, what, (depth[1][s] - depth[0][s]) / UNIT, 276.0, 2.0);
    }
    /* gl_FragCoord.z reads the biased depth of the sample */
    for (i = 0; i < sizeof(sloped) / sizeof(sloped[0]); i++)
    {
        draw_twice_ms(fbo[0], textures, fbo[1], program_fetch, program_z, &sloped[i], 0.0f, 100.0f, depth, red);
        for (s = 0; s < SAMPLES; s++)
        {
            snprintf(what, sizeof(what), "gl_FragCoord.z, %s, units 100, sample %d, depth", sloped_names[i], s);
            ok &= report(name, what, (depth[1][s] - depth[0][s]) / UNIT, 200.0, 2.0);
            snprintf(what, sizeof(what), "gl_FragCoord.z, %s, units 100, sample %d, red minus depth",
                     sloped_names[i], s);
            ok &= report(name, what, (red[1][s] - depth[1][s]) / UNIT, 0.0, 1.0);
        }
    }
    /* the covered samples of a pixel whose centre is not covered */
    draw_twice_ms(fbo[0], textures, fbo[1], program_fetch, 0, &edge, 0.0f, 100.0f, depth, red);
    for (s = covered = 0; s < SAMPLES; s++)
    {
        if (red[0][s] != 1.0f || red[1][s] != 1.0f)
            continue;
        covered++;
        snprintf(what, sizeof(what), "edge at x 32.45, slope 256 in x, units 100, sample %d", s);
        ok &= report(name, what, (depth[1][s] - depth[0][s]) / UNIT, 200.0, 2.0);
    }
    if (!covered || covered == SAMPLES)
    {
        printf("egl: depth-bias: %s: edge at x 32.45: %d of %d samples covered FAILED\n", name, covered,
               SAMPLES);
        ok = 0;
    }
    gl.BindFramebuffer(GL_FRAMEBUFFER, 0);
    return ok;
}

static int check_depth_bias(get_proc_address_t get_proc_address)
{
    void **entries = (void **)&gl;
    unsigned int program_z, program_xy, program_fetch, error;
    size_t i;
    int ok;

    for (i = 0; i < sizeof(gl_names) / sizeof(gl_names[0]); i++)
    {
        if (!(entries[i] = get_proc_address(gl_names[i])))
        {
            printf("egl: depth-bias: no %s\n", gl_names[i]);
            return FAIL_DEPTH_BIAS;
        }
    }
    if (!(program_z = make_program(vs_source, fs_z_source)) ||
        !(program_xy = make_program(vs_source, fs_xy_source)) ||
        !(program_fetch = make_program(vs_fetch_source, fs_fetch_source)))
    {
        printf("egl: depth-bias: a shader did not compile or link\n");
        return FAIL_DEPTH_BIAS;
    }
    ok = check_depth_format("GL_DEPTH_COMPONENT24", GL_DEPTH_COMPONENT24, GL_DEPTH_ATTACHMENT,
                            program_z, program_xy);
    ok &= check_depth_format("GL_DEPTH24_STENCIL8", GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT,
                             program_z, program_xy);
    ok &= check_depth_format_ms("GL_DEPTH_COMPONENT24, 4 samples", GL_DEPTH_COMPONENT24,
                                GL_DEPTH_ATTACHMENT, program_z, program_fetch);
    ok &= check_depth_format_ms("GL_DEPTH24_STENCIL8, 4 samples", GL_DEPTH24_STENCIL8,
                                GL_DEPTH_STENCIL_ATTACHMENT, program_z, program_fetch);
    if ((error = gl.GetError()))
    {
        printf("egl: depth-bias: GL error %#x\n", error);
        ok = 0;
    }
    printf("egl: depth-bias check %s\n", ok ? "ok" : "FAILED");
    return ok ? 0 : FAIL_DEPTH_BIAS;
}

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
    int depth_bias, status = 0;
    uint32_t i;
    void *egl;

    depth_bias = argc == 3 && !strcmp(argv[2], "depth-bias");
    if (argc != 2 && !depth_bias)
    {
        printf("usage: %s <libEGL.1.dylib> [depth-bias]\n", argv[0]);
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

    if (depth_bias)
        status = check_depth_bias(get_proc_address);
    for (i = 0; i < _dyld_image_count(); i++)
    {
        const char *name = _dyld_get_image_name(i);
        if (strstr(name, "libEGL") || strstr(name, "libgallium") || strstr(name, "libvulkan"))
            printf("egl: loaded %s\n", name);
    }
    make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    terminate(display);
    return status;
}
