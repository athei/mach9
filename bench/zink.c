/* GPU time of OpenGL on Zink on KosmicKrisp for a shadow map pass: depth-only
 * draws with polygon offset into a large depth texture, on a 16-bit and a
 * 24-bit depth format, with the offset off and on. Zink applies the depth
 * bias of some formats in the fragment shader, which then writes the depth
 * and loses early depth testing; the difference between the offset arms of a
 * format is what that costs on top of the device's own bias. It runs
 * natively, outside Wine, on the EGL it is given, the surfaceless platform
 * winemac.drv uses, with a core 4.3 context. Build and run it with zink.sh
 * next to it:
 *
 *     bench/zink.sh [<bundle or MESA_DIST>] [--rounds <n>]
 *
 * The stack defaults to dist/wine. A MESA_DIST of bin/build-mesa.sh compares
 * two Mesa builds without a bundle step.
 *
 * | Arm | Depth texture | glPolygonOffset |
 * | --- | --- | --- |
 * | D16 off, D16 on | GL_DEPTH_COMPONENT16 | off, (FACTOR, UNITS) |
 * | D24 off, D24 on | GL_DEPTH_COMPONENT24, which Zink stores as D32_SFLOAT on KosmicKrisp | the same |
 *
 * - The pass: a TARGET_SIZE square depth texture with no colour attachment,
 *   cleared, a ground quad over all of it at depth GROUND_Z, then CASTERS
 *   draws of one sphere mesh of SPHERE_TRIANGLES triangles each, placed,
 *   sized and ordered by a fixed pseudo-random sequence, back faces culled,
 *   GL_LESS, an empty fragment shader. Their front faces cover the texture
 *   about twice over the ground, the middle about 3.5 times. Each draw sets
 *   its placement with one glUniform4f, as an engine sets a caster's
 *   transform.
 * - The time is the GPU time of the pass, from a GL_TIME_ELAPSED query. Its
 *   begin follows a fixed load, LOAD_GROUPS workgroups of LOAD_ITERATIONS
 *   dependent multiply-adds, in the same command buffer, so the GPU is in
 *   its highest performance state when the pass starts (docs/internals.md,
 *   "Vulkan", on bench/kk.sh). Its end follows a one-workgroup dispatch,
 *   which ends Zink's render pass: in a Metal render encoder KosmicKrisp
 *   hands a timestamp the value of the one before it. The interval holds the
 *   clear, the draws, that dispatch and the timestamp resolve at the end of
 *   the render encoder (about 27 us, #117), the same in every arm.
 * - Before the timed rounds each arm runs once with a check. The centre
 *   texel holds the nearest point of the first caster, which no other caster
 *   comes near, so it shows that the front faces were drawn; a corner texel
 *   holds the ground. The difference of that corner texel between the on and
 *   off arm of a format is the constant offset in units of the format,
 *   printed with the expected 2 * UNITS (Mesa's drivers take one GL unit as
 *   two). On D16 it names the path: Metal's own bias stores one unit more
 *   (#80), Zink's shader bias the expected value. On D24 both store the
 *   expected value at the ground's depth, where Metal's float step is the
 *   24-bit unit. An arm whose check fails, and both arms of a format whose
 *   offset is more than 2 units off, print FAIL and are not timed, and the
 *   exit status is 1. Each arm also draws the pass once inside a
 *   GL_FRAGMENT_SHADER_INVOCATIONS query, printed per format: a shader that
 *   writes the depth runs for fragments that early depth testing would have
 *   rejected, so the count names the path on either format.
 * - Every round runs each arm once, starting with another arm each round,
 *   after untimed warm-up rounds that also let Zink's background pipeline
 *   compiles finish. It prints one line per arm and one per format for the
 *   difference on minus off within a round: the median, quartiles, minimum
 *   and maximum over the rounds in ms.
 * - Compare the differences of one invocation, not absolute medians across
 *   invocations. For a before and after, alternate invocations of the two
 *   stacks.
 *
 * The EGL and GL declarations it needs are written out here, so it builds
 * without EGL or GL headers. */

#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* A cascade of a shadow map; 4096 is common for the nearest one. */
#define TARGET_SIZE 4096
#define CASTERS 2048
#define SPHERE_SEGMENTS 24
#define SPHERE_RINGS 12
#define SPHERE_TRIANGLES (2 * SPHERE_SEGMENTS * (SPHERE_RINGS - 1))
/* Radii of the casters, in units of half the texture. */
#define MIN_RADIUS 0.02
#define MAX_RADIUS 0.05
/* The casters' centres are within this of the texture's centre and between
 * these depths. */
#define CENTRE_RANGE 0.75
#define MIN_DEPTH 0.2
#define MAX_DEPTH 0.85
#define GROUND_Z 0.95
/* The first caster, at the centre and nearer than every other one. */
#define FIRST_Z 0.15
#define FIRST_RADIUS 0.05
/* A common shadow map offset. */
#define FACTOR 2.0f
#define UNITS 4.0f
#define LOAD_GROUPS 16384
#define LOAD_ITERATIONS 20000
#define WARMUP_ROUNDS 5
#define DEFAULT_ROUNDS 30
#define PI 3.14159265358979323846

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

typedef EGLDisplay (*get_platform_display_t)(EGLenum, void *, const intptr_t *);
typedef EGLBoolean (*initialize_t)(EGLDisplay, EGLint *, EGLint *);
typedef EGLBoolean (*bind_api_t)(EGLenum);
typedef EGLBoolean (*choose_config_t)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *);
typedef EGLContext (*create_context_t)(EGLDisplay, EGLConfig, EGLContext, const EGLint *);
typedef EGLBoolean (*make_current_t)(EGLDisplay, void *, void *, EGLContext);
typedef EGLint (*get_error_t)(void);
typedef void *(*get_proc_address_t)(const char *);

#define GL_NONE 0
#define GL_TRIANGLES 0x0004
#define GL_DEPTH_BUFFER_BIT 0x0100
#define GL_LESS 0x0201
#define GL_BACK 0x0405
#define GL_CCW 0x0901
#define GL_CULL_FACE 0x0B44
#define GL_DEPTH_TEST 0x0B71
#define GL_TEXTURE_2D 0x0DE1
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02
#define GL_UNSIGNED_SHORT 0x1403
#define GL_FLOAT 0x1406
#define GL_DEPTH_COMPONENT 0x1902
#define GL_POLYGON_OFFSET_FILL 0x8037
#define GL_DEPTH_COMPONENT16 0x81A5
#define GL_DEPTH_COMPONENT24 0x81A6
#define GL_FRAGMENT_SHADER_INVOCATIONS 0x82F4
#define GL_QUERY_RESULT 0x8866
#define GL_ARRAY_BUFFER 0x8892
#define GL_ELEMENT_ARRAY_BUFFER 0x8893
#define GL_STATIC_DRAW 0x88E4
#define GL_DYNAMIC_COPY 0x88EA
#define GL_TIME_ELAPSED 0x88BF
#define GL_FRAGMENT_SHADER 0x8B30
#define GL_VERTEX_SHADER 0x8B31
#define GL_COMPILE_STATUS 0x8B81
#define GL_LINK_STATUS 0x8B82
#define GL_FRAMEBUFFER_COMPLETE 0x8CD5
#define GL_DEPTH_ATTACHMENT 0x8D00
#define GL_FRAMEBUFFER 0x8D40
#define GL_SHADER_STORAGE_BUFFER 0x90D2
#define GL_COMPUTE_SHADER 0x91B9

/* the GL entry points, in the order of gl_names */
static struct
{
    const unsigned char *(*GetString)(unsigned int);
    unsigned int (*GetError)(void);
    void (*Enable)(unsigned int);
    void (*Disable)(unsigned int);
    void (*Viewport)(int, int, int, int);
    void (*DepthFunc)(unsigned int);
    void (*CullFace)(unsigned int);
    void (*FrontFace)(unsigned int);
    void (*ClearDepth)(double);
    void (*Clear)(unsigned int);
    void (*PolygonOffset)(float, float);
    void (*DrawBuffer)(unsigned int);
    void (*ReadBuffer)(unsigned int);
    void (*ReadPixels)(int, int, int, int, unsigned int, unsigned int, void *);
    void (*GenTextures)(int, unsigned int *);
    void (*BindTexture)(unsigned int, unsigned int);
    void (*TexStorage2D)(unsigned int, int, unsigned int, int, int);
    void (*GenFramebuffers)(int, unsigned int *);
    void (*BindFramebuffer)(unsigned int, unsigned int);
    void (*FramebufferTexture2D)(unsigned int, unsigned int, unsigned int, unsigned int, int);
    unsigned int (*CheckFramebufferStatus)(unsigned int);
    void (*GenVertexArrays)(int, unsigned int *);
    void (*BindVertexArray)(unsigned int);
    void (*GenBuffers)(int, unsigned int *);
    void (*BindBuffer)(unsigned int, unsigned int);
    void (*BindBufferBase)(unsigned int, unsigned int, unsigned int);
    void (*BufferData)(unsigned int, intptr_t, const void *, unsigned int);
    void (*VertexAttribPointer)(unsigned int, int, unsigned int, unsigned char, int, const void *);
    void (*EnableVertexAttribArray)(unsigned int);
    void (*DrawElements)(unsigned int, int, unsigned int, const void *);
    unsigned int (*CreateShader)(unsigned int);
    void (*ShaderSource)(unsigned int, int, const char *const *, const int *);
    void (*CompileShader)(unsigned int);
    void (*GetShaderiv)(unsigned int, unsigned int, int *);
    void (*GetShaderInfoLog)(unsigned int, int, int *, char *);
    unsigned int (*CreateProgram)(void);
    void (*AttachShader)(unsigned int, unsigned int);
    void (*LinkProgram)(unsigned int);
    void (*GetProgramiv)(unsigned int, unsigned int, int *);
    void (*UseProgram)(unsigned int);
    int (*GetUniformLocation)(unsigned int, const char *);
    void (*Uniform4f)(int, float, float, float, float);
    void (*Uniform1ui)(int, unsigned int);
    void (*DispatchCompute)(unsigned int, unsigned int, unsigned int);
    void (*GenQueries)(int, unsigned int *);
    void (*BeginQuery)(unsigned int, unsigned int);
    void (*EndQuery)(unsigned int);
    void (*GetQueryObjectui64v)(unsigned int, unsigned int, uint64_t *);
} gl;

static const char *const gl_names[] = {
    "glGetString", "glGetError", "glEnable", "glDisable", "glViewport", "glDepthFunc", "glCullFace",
    "glFrontFace", "glClearDepth", "glClear", "glPolygonOffset", "glDrawBuffer", "glReadBuffer",
    "glReadPixels", "glGenTextures", "glBindTexture", "glTexStorage2D", "glGenFramebuffers",
    "glBindFramebuffer", "glFramebufferTexture2D", "glCheckFramebufferStatus", "glGenVertexArrays",
    "glBindVertexArray", "glGenBuffers", "glBindBuffer", "glBindBufferBase", "glBufferData",
    "glVertexAttribPointer", "glEnableVertexAttribArray", "glDrawElements", "glCreateShader",
    "glShaderSource", "glCompileShader", "glGetShaderiv", "glGetShaderInfoLog", "glCreateProgram",
    "glAttachShader", "glLinkProgram", "glGetProgramiv", "glUseProgram", "glGetUniformLocation",
    "glUniform4f", "glUniform1ui", "glDispatchCompute", "glGenQueries", "glBeginQuery", "glEndQuery",
    "glGetQueryObjectui64v",
};

/* place: centre x, y and depth, and radius; x and y in units of half the
 * texture, depth in window coordinates, and the sphere as deep as it is
 * wide */
static const char vs_source[] =
    "#version 430 core\n"
    "layout(location = 0) in vec3 local;\n"
    "uniform vec4 place;\n"
    "void main()\n"
    "{\n"
    "    vec3 p = place.xyz + local * place.w;\n"
    "    gl_Position = vec4(p.xy, 2.0 * p.z - 1.0, 1.0);\n"
    "}\n";
static const char fs_source[] = "#version 430 core\nvoid main()\n{\n}\n";
static const char load_source[] =
    "#version 430 core\n"
    "layout(local_size_x = 64) in;\n"
    "layout(std430, binding = 0) buffer Out { float v[]; };\n"
    "uniform uint iterations;\n"
    "void main()\n"
    "{\n"
    "    float x = float(gl_GlobalInvocationID.x);\n"
    "    for (uint i = 0u; i < iterations; i++)\n"
    "        x = fma(x, 0.999, 0.5);\n"
    "    v[gl_GlobalInvocationID.x] = x;\n"
    "}\n";

struct arm
{
    const char *name;
    unsigned int format;
    double unit; /* one step of the format, as a depth */
    bool offset;
    unsigned int texture, fbo, query;
    double centre, corner; /* the check's texels */
    uint64_t invocations;  /* of the fragment shader in one pass */
    bool ready;
    double *samples;
};

static struct arm arms[] = {
#define ARM(arm_name, arm_format, arm_unit, arm_offset)                                            \
    { .name = arm_name, .format = arm_format, .unit = arm_unit, .offset = arm_offset }
    ARM("D16 off", GL_DEPTH_COMPONENT16, 1.0 / 65535, false),
    ARM("D16 on", GL_DEPTH_COMPONENT16, 1.0 / 65535, true),
    ARM("D24 off", GL_DEPTH_COMPONENT24, 1.0 / (1 << 24), false),
    ARM("D24 on", GL_DEPTH_COMPONENT24, 1.0 / (1 << 24), true),
#undef ARM
};
#define ARM_COUNT (sizeof(arms) / sizeof(arms[0]))

static unsigned int shadow_program, load_program;
static int place_location, iterations_location;
static unsigned int index_count, ground_first;
static float places[CASTERS][4];

static unsigned int compile(unsigned int type, const char *source)
{
    unsigned int shader = gl.CreateShader(type);
    char log[1024] = "";
    int ok;

    gl.ShaderSource(shader, 1, &source, NULL);
    gl.CompileShader(shader);
    gl.GetShaderiv(shader, GL_COMPILE_STATUS, &ok);
    if (ok) return shader;
    gl.GetShaderInfoLog(shader, sizeof(log), NULL, log);
    printf("zink: a shader did not compile: %s\n", log);
    return 0;
}

static unsigned int link(unsigned int a, unsigned int b)
{
    unsigned int program = gl.CreateProgram();
    int ok;

    gl.AttachShader(program, a);
    if (b) gl.AttachShader(program, b);
    gl.LinkProgram(program);
    gl.GetProgramiv(program, GL_LINK_STATUS, &ok);
    if (ok) return program;
    printf("zink: a program did not link\n");
    return 0;
}

static uint32_t seed = 1;

static double next_random(void)
{
    seed = seed * 1664525u + 1013904223u;
    return (seed >> 8) / 16777216.0;
}

/* The sphere around the z axis, its poles at local z -1 and 1, wound so that
 * the faces towards the viewer (lower z) are counter-clockwise in the window
 * and those away from it clockwise; then the ground quad at local z 0,
 * counter-clockwise. */
static bool create_meshes(void)
{
    static float vertices[(SPHERE_RINGS + 1) * (SPHERE_SEGMENTS + 1) + 4][3];
    static uint16_t indices[SPHERE_TRIANGLES * 3 + 6];
    static const float ground[4][3] = { { -1, -1, 0 }, { -1, 1, 0 }, { 1, 1, 0 }, { 1, -1, 0 } };
    unsigned int vao, buffers[2], ring, segment, v = 0, n = 0, i;

    for (ring = 0; ring <= SPHERE_RINGS; ring++)
    {
        double theta = PI * ring / SPHERE_RINGS;
        for (segment = 0; segment <= SPHERE_SEGMENTS; segment++, v++)
        {
            double phi = 2 * PI * segment / SPHERE_SEGMENTS;
            vertices[v][0] = (float)(sin(theta) * cos(phi));
            vertices[v][1] = (float)(sin(theta) * sin(phi));
            vertices[v][2] = (float)-cos(theta);
        }
    }
    for (ring = 0; ring < SPHERE_RINGS; ring++)
    {
        for (segment = 0; segment < SPHERE_SEGMENTS; segment++)
        {
            uint16_t a = ring * (SPHERE_SEGMENTS + 1) + segment, b = a + 1;
            uint16_t c = a + SPHERE_SEGMENTS + 1, d = c + 1;
            /* the triangles at the poles have two corners in one point */
            if (ring)
            {
                indices[n++] = a;
                indices[n++] = d;
                indices[n++] = b;
            }
            if (ring + 1 < SPHERE_RINGS)
            {
                indices[n++] = a;
                indices[n++] = c;
                indices[n++] = d;
            }
        }
    }
    index_count = n;
    ground_first = n;
    for (i = 0; i < 4; i++) memcpy(vertices[v + i], ground[i], sizeof(ground[i]));
    indices[n++] = v;
    indices[n++] = v + 2;
    indices[n++] = v + 1;
    indices[n++] = v;
    indices[n++] = v + 3;
    indices[n++] = v + 2;

    gl.GenVertexArrays(1, &vao);
    gl.BindVertexArray(vao);
    gl.GenBuffers(2, buffers);
    gl.BindBuffer(GL_ARRAY_BUFFER, buffers[0]);
    gl.BufferData(GL_ARRAY_BUFFER, sizeof(vertices), vertices, GL_STATIC_DRAW);
    gl.VertexAttribPointer(0, 3, GL_FLOAT, 0, sizeof(vertices[0]), NULL);
    gl.EnableVertexAttribArray(0);
    gl.BindBuffer(GL_ELEMENT_ARRAY_BUFFER, buffers[1]);
    gl.BufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(indices), indices, GL_STATIC_DRAW);
    return index_count == SPHERE_TRIANGLES * 3;
}

static void place_casters(void)
{
    unsigned int i;

    places[0][0] = places[0][1] = 0.0f;
    places[0][2] = FIRST_Z;
    places[0][3] = FIRST_RADIUS;
    for (i = 1; i < CASTERS; i++)
    {
        places[i][0] = (float)((2 * next_random() - 1) * CENTRE_RANGE);
        places[i][1] = (float)((2 * next_random() - 1) * CENTRE_RANGE);
        places[i][2] = (float)(MIN_DEPTH + next_random() * (MAX_DEPTH - MIN_DEPTH));
        places[i][3] = (float)(MIN_RADIUS + next_random() * (MAX_RADIUS - MIN_RADIUS));
    }
}

static void dispatch_load(unsigned int groups, unsigned int iterations)
{
    gl.UseProgram(load_program);
    gl.Uniform1ui(iterations_location, iterations);
    gl.DispatchCompute(groups, 1, 1);
}

static void draw_pass(struct arm *arm)
{
    unsigned int i;

    gl.BindFramebuffer(GL_FRAMEBUFFER, arm->fbo);
    gl.Viewport(0, 0, TARGET_SIZE, TARGET_SIZE);
    gl.ClearDepth(1.0);
    gl.Clear(GL_DEPTH_BUFFER_BIT);
    gl.Enable(GL_DEPTH_TEST);
    gl.DepthFunc(GL_LESS);
    gl.Enable(GL_CULL_FACE);
    gl.CullFace(GL_BACK);
    gl.FrontFace(GL_CCW);
    if (arm->offset)
    {
        gl.Enable(GL_POLYGON_OFFSET_FILL);
        gl.PolygonOffset(FACTOR, UNITS);
    }
    else
        gl.Disable(GL_POLYGON_OFFSET_FILL);
    gl.UseProgram(shadow_program);
    gl.Uniform4f(place_location, 0.0f, 0.0f, (float)GROUND_Z, 1.0f);
    gl.DrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (const void *)(ground_first * sizeof(uint16_t)));
    for (i = 0; i < CASTERS; i++)
    {
        gl.Uniform4f(place_location, places[i][0], places[i][1], places[i][2], places[i][3]);
        gl.DrawElements(GL_TRIANGLES, index_count, GL_UNSIGNED_SHORT, NULL);
    }
    gl.Disable(GL_POLYGON_OFFSET_FILL);
    gl.Disable(GL_CULL_FACE);
    gl.Disable(GL_DEPTH_TEST);
    gl.BindFramebuffer(GL_FRAMEBUFFER, 0);
}

/* The GPU time of one pass in ms, after the load; 0 when the query gave
 * none. */
static double run(struct arm *arm)
{
    uint64_t ns = 0;

    dispatch_load(LOAD_GROUPS, LOAD_ITERATIONS);
    gl.BeginQuery(GL_TIME_ELAPSED, arm->query);
    draw_pass(arm);
    dispatch_load(1, 1);
    gl.EndQuery(GL_TIME_ELAPSED);
    gl.GetQueryObjectui64v(arm->query, GL_QUERY_RESULT, &ns);
    return ns / 1e6;
}

static double read_depth(struct arm *arm, int x, int y)
{
    float depth = -1.0f;

    gl.BindFramebuffer(GL_FRAMEBUFFER, arm->fbo);
    gl.ReadPixels(x, y, 1, 1, GL_DEPTH_COMPONENT, GL_FLOAT, &depth);
    gl.BindFramebuffer(GL_FRAMEBUFFER, 0);
    return depth;
}

static bool setup(struct arm *arm)
{
    /* the nearest point of the first caster, plus the constant offset; the
     * slope term of the pole triangle there (a few D16 units) is small
     * against the tolerance below, which only has to tell the front face
     * from the back face, 2 * FIRST_RADIUS further */
    double want = FIRST_Z - FIRST_RADIUS + (arm->offset ? 2 * UNITS * arm->unit : 0.0);
    unsigned int error, stats;

    gl.GenTextures(1, &arm->texture);
    gl.BindTexture(GL_TEXTURE_2D, arm->texture);
    gl.TexStorage2D(GL_TEXTURE_2D, 1, arm->format, TARGET_SIZE, TARGET_SIZE);
    gl.BindTexture(GL_TEXTURE_2D, 0);
    gl.GenFramebuffers(1, &arm->fbo);
    gl.BindFramebuffer(GL_FRAMEBUFFER, arm->fbo);
    gl.FramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, arm->texture, 0);
    gl.DrawBuffer(GL_NONE);
    gl.ReadBuffer(GL_NONE);
    if (gl.CheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    {
        printf("zink: FAIL %s: framebuffer incomplete\n", arm->name);
        return false;
    }
    gl.BindFramebuffer(GL_FRAMEBUFFER, 0);
    gl.GenQueries(1, &arm->query);

    gl.GenQueries(1, &stats);
    gl.BeginQuery(GL_FRAGMENT_SHADER_INVOCATIONS, stats);
    draw_pass(arm);
    gl.EndQuery(GL_FRAGMENT_SHADER_INVOCATIONS);
    gl.GetQueryObjectui64v(stats, GL_QUERY_RESULT, &arm->invocations);

    if (run(arm) <= 0.0)
    {
        printf("zink: FAIL %s: the GL_TIME_ELAPSED query gave no time\n", arm->name);
        return false;
    }
    arm->centre = read_depth(arm, TARGET_SIZE / 2, TARGET_SIZE / 2);
    arm->corner = read_depth(arm, 2, 2);
    if ((error = gl.GetError()))
    {
        printf("zink: FAIL %s: GL error %#x\n", arm->name, error);
        return false;
    }
    /* a back face would be FIRST_RADIUS * 2 further */
    if (fabs(arm->centre - want) > 0.002)
    {
        printf("zink: FAIL %s: the centre texel is %.6f, not about %.6f\n", arm->name, arm->centre, want);
        return false;
    }
    return true;
}

/* The ground's offset of an on arm in units of its format, against its
 * format's off arm; false when it is not about the expected one. */
static bool check_offset(const struct arm *on, const struct arm *off)
{
    double units = (on->corner - off->corner) / on->unit, want = 2 * UNITS;
    bool ok = fabs(units - want) <= 2.0;

    printf("zink: %s: ground offset %+.2f units of the format, expected %+.2f%s\n", on->name, units,
           want, ok ? "" : " FAIL");
    return ok;
}

static int compare(const void *a, const void *b)
{
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}

/* The <p> quantile of the <n> sorted <samples>, interpolated between the two
 * nearest. */
static double quantile(const double *samples, unsigned n, double p)
{
    double pos = p * (n - 1);
    unsigned below = (unsigned)pos;

    if (below + 1 >= n) return samples[n - 1];
    return samples[below] + (pos - below) * (samples[below + 1] - samples[below]);
}

static void report(const char *name, double *s, unsigned rounds)
{
    qsort(s, rounds, sizeof(double), compare);
    printf("shadow-pass %s: median %.3f ms, quartiles %.3f to %.3f, min %.3f, max %.3f, n=%u\n", name,
           quantile(s, rounds, 0.5), quantile(s, rounds, 0.25), quantile(s, rounds, 0.75), s[0],
           s[rounds - 1], rounds);
}

static bool create_context(const char *path)
{
    static const EGLint config_attribs[] = { EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE };
    static const EGLint context_attribs[] = { EGL_CONTEXT_MAJOR_VERSION, 4, EGL_CONTEXT_MINOR_VERSION, 3,
                                              EGL_CONTEXT_OPENGL_PROFILE_MASK,
                                              EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT, EGL_NONE };
    get_platform_display_t get_platform_display;
    initialize_t initialize;
    bind_api_t bind_api;
    choose_config_t choose_config;
    create_context_t create;
    make_current_t make_current;
    get_error_t get_error;
    get_proc_address_t get_proc_address;
    EGLint major, minor, count = 0;
    EGLDisplay display;
    EGLContext context;
    EGLConfig config;
    void **entries = (void **)&gl;
    void *egl;
    size_t i;

    if (!(egl = dlopen(path, RTLD_NOW | RTLD_LOCAL)))
    {
        printf("zink: cannot load %s: %s\n", path, dlerror());
        return false;
    }
    get_platform_display = (get_platform_display_t)dlsym(egl, "eglGetPlatformDisplay");
    initialize = (initialize_t)dlsym(egl, "eglInitialize");
    bind_api = (bind_api_t)dlsym(egl, "eglBindAPI");
    choose_config = (choose_config_t)dlsym(egl, "eglChooseConfig");
    create = (create_context_t)dlsym(egl, "eglCreateContext");
    make_current = (make_current_t)dlsym(egl, "eglMakeCurrent");
    get_error = (get_error_t)dlsym(egl, "eglGetError");
    get_proc_address = (get_proc_address_t)dlsym(egl, "eglGetProcAddress");
    if (!get_platform_display || !initialize || !bind_api || !choose_config || !create ||
        !make_current || !get_error || !get_proc_address)
    {
        printf("zink: %s lacks an EGL 1.5 entry point\n", path);
        return false;
    }
    display = get_platform_display(EGL_PLATFORM_SURFACELESS_MESA, NULL, NULL);
    if (!display || !initialize(display, &major, &minor) || !bind_api(EGL_OPENGL_API) ||
        !choose_config(display, config_attribs, &config, 1, &count) || !count ||
        !(context = create(display, config, EGL_NO_CONTEXT, context_attribs)) ||
        !make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, context))
    {
        printf("zink: no OpenGL 4.3 core context on the surfaceless platform, error %#x\n", get_error());
        return false;
    }
    for (i = 0; i < sizeof(gl_names) / sizeof(gl_names[0]); i++)
    {
        if (!(entries[i] = get_proc_address(gl_names[i])))
        {
            printf("zink: no %s\n", gl_names[i]);
            return false;
        }
    }
    printf("zink: GL_RENDERER %s\n", (const char *)gl.GetString(GL_RENDERER));
    printf("zink: GL_VERSION %s\n", (const char *)gl.GetString(GL_VERSION));
    for (i = 0; i < _dyld_image_count(); i++)
    {
        const char *name = _dyld_get_image_name(i);
        if (strstr(name, "libEGL") || strstr(name, "libgallium") || strstr(name, "libvulkan"))
            printf("zink: loaded %s\n", name);
    }
    return true;
}

static bool create_programs(void)
{
    unsigned int vs, fs, cs, buffer;

    if (!(vs = compile(GL_VERTEX_SHADER, vs_source)) || !(fs = compile(GL_FRAGMENT_SHADER, fs_source)) ||
        !(cs = compile(GL_COMPUTE_SHADER, load_source)) || !(shadow_program = link(vs, fs)) ||
        !(load_program = link(cs, 0)))
        return false;
    place_location = gl.GetUniformLocation(shadow_program, "place");
    iterations_location = gl.GetUniformLocation(load_program, "iterations");
    gl.GenBuffers(1, &buffer);
    gl.BindBuffer(GL_SHADER_STORAGE_BUFFER, buffer);
    gl.BufferData(GL_SHADER_STORAGE_BUFFER, LOAD_GROUPS * 64 * sizeof(float), NULL, GL_DYNAMIC_COPY);
    gl.BindBufferBase(GL_SHADER_STORAGE_BUFFER, 0, buffer);
    return place_location >= 0 && iterations_location >= 0;
}

/* The GPU time of the load alone, printed so its size can be checked
 * against the 16 ms the GPU needs to reach its highest state. */
static void time_load(void)
{
    unsigned int query;
    uint64_t ns = 0;

    gl.GenQueries(1, &query);
    dispatch_load(1, 1);
    gl.BeginQuery(GL_TIME_ELAPSED, query);
    dispatch_load(LOAD_GROUPS, LOAD_ITERATIONS);
    gl.EndQuery(GL_TIME_ELAPSED);
    gl.GetQueryObjectui64v(query, GL_QUERY_RESULT, &ns);
    printf("zink: the load takes %.3f ms\n", ns / 1e6);
}

static void usage(const char *argv0)
{
    printf("usage: %s <libEGL.1.dylib> [--rounds <n>]\n", argv0);
}

int main(int argc, char **argv)
{
    unsigned rounds = DEFAULT_ROUNDS, round;
    struct arm *order[ARM_COUNT];
    bool failed = false;
    size_t i, j, selected = 0;
    double *diffs[ARM_COUNT / 2];

    if (argc == 4 && !strcmp(argv[2], "--rounds"))
    {
        char *end;
        unsigned long value = strtoul(argv[3], &end, 10);
        if (*end || !value || value > 100000)
        {
            usage(argv[0]);
            return 2;
        }
        rounds = (unsigned)value;
    }
    else if (argc != 2)
    {
        usage(argv[0]);
        return 2;
    }
    if (!create_context(argv[1]) || !create_programs() || !create_meshes())
        return 1;
    printf("zink: %ux%u depth texture, %u casters of %u triangles, glPolygonOffset(%g, %g)\n",
           TARGET_SIZE, TARGET_SIZE, CASTERS, SPHERE_TRIANGLES, FACTOR, UNITS);
    place_casters();
    time_load();

    for (i = 0; i < ARM_COUNT; i++)
    {
        struct arm *arm = &arms[i];
        if (!setup(arm))
        {
            printf("shadow-pass %s: FAIL, not timed\n", arm->name);
            failed = true;
            continue;
        }
        if (!(arm->samples = calloc(rounds, sizeof(double))))
        {
            printf("zink: out of memory\n");
            return 1;
        }
        arm->ready = true;
    }
    /* arms come in pairs of one format, off then on; a pair is timed only
     * when both arms are ready and the offset is the one the on arm sets */
    for (i = 0; i < ARM_COUNT; i += 2)
    {
        if (!arms[i].ready || !arms[i + 1].ready)
        {
            arms[i].ready = arms[i + 1].ready = false;
            continue;
        }
        printf("zink: %s: ground %.6f, centre %.6f\n", arms[i].name, arms[i].corner, arms[i].centre);
        printf("zink: %s, %s: fragment shader invocations %llu, %llu\n", arms[i].name, arms[i + 1].name,
               (unsigned long long)arms[i].invocations, (unsigned long long)arms[i + 1].invocations);
        if (!check_offset(&arms[i + 1], &arms[i]))
        {
            printf("shadow-pass %.3s: FAIL, not timed\n", arms[i].name);
            arms[i].ready = arms[i + 1].ready = false;
            failed = true;
            continue;
        }
        order[selected++] = &arms[i];
        order[selected++] = &arms[i + 1];
    }

    /* Round r starts with the (r mod n)th arm and keeps the order, so each
     * round has another first arm. */
    for (round = 0; round < WARMUP_ROUNDS + rounds; round++)
    {
        for (j = 0; j < selected; j++)
        {
            struct arm *arm = order[(round + j) % selected];
            double ms = run(arm);
            if (ms <= 0.0)
            {
                printf("zink: %s gave no time in round %u\n", arm->name, round);
                return 1;
            }
            if (round >= WARMUP_ROUNDS) arm->samples[round - WARMUP_ROUNDS] = ms;
        }
    }

    for (i = 0; i < ARM_COUNT; i += 2)
    {
        char name[64];
        if (!arms[i].ready || !arms[i + 1].ready) continue;
        if (!(diffs[i / 2] = calloc(rounds, sizeof(double))))
        {
            printf("zink: out of memory\n");
            return 1;
        }
        for (round = 0; round < rounds; round++)
            diffs[i / 2][round] = arms[i + 1].samples[round] - arms[i].samples[round];
        report(arms[i].name, arms[i].samples, rounds);
        report(arms[i + 1].name, arms[i + 1].samples, rounds);
        snprintf(name, sizeof(name), "%.3s on - off", arms[i].name);
        report(name, diffs[i / 2], rounds);
    }
    return failed;
}
