/* The host program bundle-wine.sh builds to check OpenGL's pipeline
 * statistics queries on Zink on KosmicKrisp, outside Wine, on a Mac with a
 * Metal 4 GPU. It makes a core context current on the libEGL.1.dylib it is
 * given, as smoke/egl.c does, and runs several GL queries at once, as
 * wined3d does for a Direct3D pipeline statistics query: statistics queries
 * of different targets, begun and ended at different times around draws,
 * beside GL_PRIMITIVES_GENERATED, which Zink counts with a statistics query
 * on KosmicKrisp, also with a geometry shader and with transform feedback
 * beside GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, and beside the compute
 * shader counter. Vulkan allows one active statistics query at a time, so
 * Zink has to count them all with one. It also runs occlusion queries over
 * many frames, more than KosmicKrisp has slots for (test_query_slots), and
 * keeps more of them alive at once than it has slots for
 * (test_query_spill).
 * Each count that the draws and dispatches fix is compared exactly; counters
 * a driver may count differently (fragment and vertex shader invocations,
 * clipping output) only have to be nonzero. The results are read with
 * glGetQueryObject and through a query buffer object, whose 32-bit writes
 * must not touch the bytes around them. It prints one line per wrong value
 * and exits with the number of them, at most 100; a status above 100 names a
 * setup step that failed.
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
typedef unsigned char GLboolean;
typedef char GLchar;
typedef intptr_t GLintptr;
typedef intptr_t GLsizeiptr;
typedef uint64_t GLuint64;
typedef float GLfloat;

#define GL_LINE_LOOP 0x0002
#define GL_TRIANGLES 0x0004
#define GL_FLOAT 0x1406
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02
#define GL_RGBA8 0x8058
#define GL_ARRAY_BUFFER 0x8892
#define GL_STATIC_DRAW 0x88E4
#define GL_STREAM_READ 0x88E1
#define GL_QUERY_RESULT 0x8866
#define GL_QUERY_RESULT_AVAILABLE 0x8867
#define GL_FRAGMENT_SHADER 0x8B30
#define GL_VERTEX_SHADER 0x8B31
#define GL_COMPILE_STATUS 0x8B81
#define GL_LINK_STATUS 0x8B82
#define GL_PRIMITIVES_GENERATED 0x8C87
#define GL_FRAMEBUFFER_COMPLETE 0x8CD5
#define GL_COLOR_ATTACHMENT0 0x8CE0
#define GL_FRAMEBUFFER 0x8D40
#define GL_RENDERBUFFER 0x8D41
#define GL_COMPUTE_SHADER 0x91B9
#define GL_QUERY_BUFFER 0x9192
#define GL_VERTICES_SUBMITTED 0x82EE
#define GL_PRIMITIVES_SUBMITTED 0x82EF
#define GL_VERTEX_SHADER_INVOCATIONS 0x82F0
#define GL_FRAGMENT_SHADER_INVOCATIONS 0x82F4
#define GL_COMPUTE_SHADER_INVOCATIONS 0x82F5
#define GL_CLIPPING_INPUT_PRIMITIVES 0x82F6
#define GL_CLIPPING_OUTPUT_PRIMITIVES 0x82F7
#define GL_GEOMETRY_SHADER_PRIMITIVES_EMITTED 0x82F3
#define GL_GEOMETRY_SHADER_INVOCATIONS 0x887F
#define GL_GEOMETRY_SHADER 0x8DD9
#define GL_INTERLEAVED_ATTRIBS 0x8C8C
#define GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN 0x8C88
#define GL_TRANSFORM_FEEDBACK_BUFFER 0x8C8E
#define GL_SAMPLES_PASSED 0x8914
#define GL_SCISSOR_TEST 0x0C11
#define GL_COLOR_BUFFER_BIT 0x4000
#define GL_RGBA 0x1908
#define GL_UNSIGNED_BYTE 0x1401
#define GL_QUERY_WAIT 0x8E13

#define GL_FUNCS \
    X(void, glGenQueries, (GLsizei, GLuint *)) \
    X(void, glDeleteQueries, (GLsizei, const GLuint *)) \
    X(void, glBeginQuery, (GLenum, GLuint)) \
    X(void, glEndQuery, (GLenum)) \
    X(void, glGetQueryObjectui64v, (GLuint, GLenum, GLuint64 *)) \
    X(void, glGetQueryObjectuiv, (GLuint, GLenum, GLuint *)) \
    X(void, glGenBuffers, (GLsizei, GLuint *)) \
    X(void, glBindBuffer, (GLenum, GLuint)) \
    X(void, glBufferData, (GLenum, GLsizeiptr, const void *, GLenum)) \
    X(void, glGetBufferSubData, (GLenum, GLintptr, GLsizeiptr, void *)) \
    X(void, glGenVertexArrays, (GLsizei, GLuint *)) \
    X(void, glBindVertexArray, (GLuint)) \
    X(void, glEnableVertexAttribArray, (GLuint)) \
    X(void, glVertexAttribPointer, (GLuint, GLint, GLenum, GLboolean, GLsizei, const void *)) \
    X(void, glGenFramebuffers, (GLsizei, GLuint *)) \
    X(void, glBindFramebuffer, (GLenum, GLuint)) \
    X(void, glGenRenderbuffers, (GLsizei, GLuint *)) \
    X(void, glBindRenderbuffer, (GLenum, GLuint)) \
    X(void, glRenderbufferStorage, (GLenum, GLenum, GLsizei, GLsizei)) \
    X(void, glFramebufferRenderbuffer, (GLenum, GLenum, GLenum, GLuint)) \
    X(GLenum, glCheckFramebufferStatus, (GLenum)) \
    X(void, glViewport, (GLint, GLint, GLsizei, GLsizei)) \
    X(GLuint, glCreateShader, (GLenum)) \
    X(void, glShaderSource, (GLuint, GLsizei, const GLchar *const *, const GLint *)) \
    X(void, glCompileShader, (GLuint)) \
    X(void, glGetShaderiv, (GLuint, GLenum, GLint *)) \
    X(GLuint, glCreateProgram, (void)) \
    X(void, glAttachShader, (GLuint, GLuint)) \
    X(void, glLinkProgram, (GLuint)) \
    X(void, glGetProgramiv, (GLuint, GLenum, GLint *)) \
    X(void, glUseProgram, (GLuint)) \
    X(void, glDrawArrays, (GLenum, GLint, GLsizei)) \
    X(void, glDispatchCompute, (GLuint, GLuint, GLuint)) \
    X(void, glTransformFeedbackVaryings, (GLuint, GLsizei, const GLchar *const *, GLenum)) \
    X(void, glBindBufferBase, (GLenum, GLuint, GLuint)) \
    X(void, glBeginTransformFeedback, (GLenum)) \
    X(void, glEndTransformFeedback, (void)) \
    X(void, glFinish, (void)) \
    X(void, glFlush, (void)) \
    X(void, glEnable, (GLenum)) \
    X(void, glDisable, (GLenum)) \
    X(void, glScissor, (GLint, GLint, GLsizei, GLsizei)) \
    X(void, glClearColor, (GLfloat, GLfloat, GLfloat, GLfloat)) \
    X(void, glClear, (GLuint)) \
    X(void, glReadPixels, (GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, void *)) \
    X(void, glBeginConditionalRender, (GLuint, GLenum)) \
    X(void, glEndConditionalRender, (void)) \
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
    FAIL_SETUP,
};

/* The draws: one, two or three triangles inside the viewport, so nothing is
 * clipped, and a line loop of four vertices. */
static const GLfloat vertices[] =
{
    -0.9f, -0.9f, -0.5f, -0.9f, -0.9f, -0.5f,
    -0.1f, -0.9f, 0.3f, -0.9f, -0.1f, -0.5f,
    0.4f, 0.1f, 0.8f, 0.1f, 0.4f, 0.5f,
    -0.5f, 0.2f, -0.2f, 0.2f, -0.2f, 0.6f, -0.5f, 0.6f,
};
#define LOOP_FIRST 9

static GLuint gfx_program, compute_program, geometry_program, xfb_program, cover_program;
static unsigned int failures;

static void report(const char *test, const char *what, GLuint64 got, const char *want)
{
    printf("glstats: %s: %s is %llu, expected %s\n", test, what, (unsigned long long)got, want);
    failures++;
}

static GLuint64 result(GLuint query)
{
    GLuint64 value = ~(GLuint64)0;
    p_glGetQueryObjectui64v(query, GL_QUERY_RESULT, &value);
    return value;
}

static void expect(const char *test, const char *what, GLuint query, GLuint64 want)
{
    GLuint64 got = result(query);
    char text[32];

    if (got != want)
    {
        snprintf(text, sizeof(text), "%llu", (unsigned long long)want);
        report(test, what, got, text);
    }
}

static void expect_nonzero(const char *test, const char *what, GLuint query)
{
    GLuint64 got = result(query);

    if (!got || got == ~(GLuint64)0)
        report(test, what, got, "a count above 0");
}

static void draw(GLenum mode, GLint first, GLsizei count)
{
    p_glUseProgram(gfx_program);
    p_glDrawArrays(mode, first, count);
}

static void dispatch(void)
{
    p_glUseProgram(compute_program);
    p_glDispatchCompute(4, 1, 1);
}

static void check_error(const char *test)
{
    GLenum error = p_glGetError();

    if (error)
    {
        printf("glstats: %s: GL error %#x\n", test, error);
        failures++;
    }
}

/* One query alone: what Zink did right before. */
static void test_single(void)
{
    GLuint q;

    p_glGenQueries(1, &q);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q);
    draw(GL_TRIANGLES, 0, 9);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    expect("single", "GL_VERTICES_SUBMITTED", q, 9);
    check_error("single");
}

/* What piglit's arb_pipeline_statistics_query-vert does: three counters
 * around one draw, plus the fragment shader counter. */
static void test_together(void)
{
    GLuint q[4];

    p_glGenQueries(4, q);
    p_glBeginQuery(GL_PRIMITIVES_SUBMITTED, q[0]);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[1]);
    p_glBeginQuery(GL_VERTEX_SHADER_INVOCATIONS, q[2]);
    p_glBeginQuery(GL_FRAGMENT_SHADER_INVOCATIONS, q[3]);
    draw(GL_TRIANGLES, 0, 9);
    p_glEndQuery(GL_PRIMITIVES_SUBMITTED);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    p_glEndQuery(GL_VERTEX_SHADER_INVOCATIONS);
    p_glEndQuery(GL_FRAGMENT_SHADER_INVOCATIONS);
    expect("together", "GL_PRIMITIVES_SUBMITTED", q[0], 3);
    expect("together", "GL_VERTICES_SUBMITTED", q[1], 9);
    expect_nonzero("together", "GL_VERTEX_SHADER_INVOCATIONS", q[2]);
    expect_nonzero("together", "GL_FRAGMENT_SHADER_INVOCATIONS", q[3]);
    check_error("together");
}

/* Queries that begin and end between draws, inside one render pass: each
 * counts only the draws it was active for. */
static void test_staggered(void)
{
    GLuint q[3];

    p_glGenQueries(3, q);
    p_glBeginQuery(GL_CLIPPING_INPUT_PRIMITIVES, q[2]);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[0]);
    draw(GL_TRIANGLES, 0, 3);
    p_glBeginQuery(GL_PRIMITIVES_SUBMITTED, q[1]);
    draw(GL_TRIANGLES, 3, 6);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    draw(GL_TRIANGLES, 0, 9);
    p_glEndQuery(GL_PRIMITIVES_SUBMITTED);
    p_glEndQuery(GL_CLIPPING_INPUT_PRIMITIVES);
    expect("staggered", "GL_VERTICES_SUBMITTED", q[0], 9);
    expect("staggered", "GL_PRIMITIVES_SUBMITTED", q[1], 5);
    expect("staggered", "GL_CLIPPING_INPUT_PRIMITIVES", q[2], 6);
    check_error("staggered");
}

/* What piglit's arb_pipeline_statistics_query-clip does. */
static void test_clip(void)
{
    GLuint q[2];

    p_glGenQueries(2, q);
    p_glBeginQuery(GL_CLIPPING_INPUT_PRIMITIVES, q[0]);
    p_glBeginQuery(GL_CLIPPING_OUTPUT_PRIMITIVES, q[1]);
    draw(GL_TRIANGLES, 0, 9);
    p_glEndQuery(GL_CLIPPING_INPUT_PRIMITIVES);
    p_glEndQuery(GL_CLIPPING_OUTPUT_PRIMITIVES);
    expect("clip", "GL_CLIPPING_INPUT_PRIMITIVES", q[0], 3);
    expect_nonzero("clip", "GL_CLIPPING_OUTPUT_PRIMITIVES", q[1]);
    check_error("clip");
}

/* GL_PRIMITIVES_GENERATED beside statistics queries. */
static void test_generated(void)
{
    GLuint q[3];

    p_glGenQueries(3, q);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[1]);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[0]);
    p_glBeginQuery(GL_CLIPPING_INPUT_PRIMITIVES, q[2]);
    draw(GL_TRIANGLES, 0, 9);
    p_glEndQuery(GL_CLIPPING_INPUT_PRIMITIVES);
    draw(GL_TRIANGLES, 0, 3);
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    expect("generated", "GL_PRIMITIVES_GENERATED", q[0], 4);
    expect("generated", "GL_VERTICES_SUBMITTED", q[1], 12);
    expect("generated", "GL_CLIPPING_INPUT_PRIMITIVES", q[2], 3);
    check_error("generated");
}

/* The compute shader counter beside a draw counter: dispatches run outside
 * render passes, draws inside them. Once begun together before any work,
 * once with the compute query begun after a draw. */
static void test_compute(void)
{
    GLuint q[4];

    p_glGenQueries(4, q);
    p_glBeginQuery(GL_COMPUTE_SHADER_INVOCATIONS, q[0]);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[1]);
    dispatch();
    draw(GL_TRIANGLES, 0, 9);
    dispatch();
    draw(GL_TRIANGLES, 0, 3);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    p_glEndQuery(GL_COMPUTE_SHADER_INVOCATIONS);
    expect("compute", "GL_COMPUTE_SHADER_INVOCATIONS", q[0], 64);
    expect("compute", "GL_VERTICES_SUBMITTED", q[1], 12);

    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[3]);
    draw(GL_TRIANGLES, 0, 9);
    p_glBeginQuery(GL_COMPUTE_SHADER_INVOCATIONS, q[2]);
    draw(GL_TRIANGLES, 0, 3);
    dispatch();
    draw(GL_TRIANGLES, 0, 6);
    p_glEndQuery(GL_COMPUTE_SHADER_INVOCATIONS);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    expect("compute later", "GL_COMPUTE_SHADER_INVOCATIONS", q[2], 32);
    expect("compute later", "GL_VERTICES_SUBMITTED", q[3], 18);
    check_error("compute");
}

/* A line loop, whose vertices Zink halves. */
static void test_line_loop(void)
{
    GLuint q[2];

    p_glGenQueries(2, q);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[0]);
    p_glBeginQuery(GL_PRIMITIVES_SUBMITTED, q[1]);
    draw(GL_LINE_LOOP, LOOP_FIRST, 4);
    draw(GL_TRIANGLES, 0, 3);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    p_glEndQuery(GL_PRIMITIVES_SUBMITTED);
    expect("line loop", "GL_VERTICES_SUBMITTED", q[0], 7);
    expect("line loop", "GL_PRIMITIVES_SUBMITTED", q[1], 5);
    check_error("line loop");
}

/* A geometry shader that emits every triangle twice, for one draw of three
 * triangles beside a draw without it: GL_PRIMITIVES_GENERATED counts what
 * the geometry shader emits where there is one. */
static void test_geometry(void)
{
    GLuint q[4];

    p_glGenQueries(4, q);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[0]);
    p_glBeginQuery(GL_GEOMETRY_SHADER_INVOCATIONS, q[1]);
    p_glBeginQuery(GL_GEOMETRY_SHADER_PRIMITIVES_EMITTED, q[2]);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[3]);
    p_glUseProgram(geometry_program);
    p_glDrawArrays(GL_TRIANGLES, 0, 9);
    draw(GL_TRIANGLES, 0, 9);
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glEndQuery(GL_GEOMETRY_SHADER_INVOCATIONS);
    p_glEndQuery(GL_GEOMETRY_SHADER_PRIMITIVES_EMITTED);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    expect("geometry", "GL_PRIMITIVES_GENERATED", q[0], 9);
    expect("geometry", "GL_GEOMETRY_SHADER_INVOCATIONS", q[1], 3);
    expect("geometry", "GL_GEOMETRY_SHADER_PRIMITIVES_EMITTED", q[2], 6);
    expect("geometry", "GL_VERTICES_SUBMITTED", q[3], 18);
    check_error("geometry");
}

/* GL_PRIMITIVES_GENERATED and a statistics query around transform
 * feedback. */
static void test_transform_feedback(void)
{
    GLuint q[3], buffer;

    p_glGenQueries(3, q);
    p_glGenBuffers(1, &buffer);
    p_glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, buffer);
    /* room for more vertices than the draws write */
    p_glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, 64 * 16, NULL, GL_STREAM_READ);
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, buffer);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[0]);
    p_glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[1]);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[2]);
    p_glUseProgram(xfb_program);
    p_glBeginTransformFeedback(GL_TRIANGLES);
    p_glDrawArrays(GL_TRIANGLES, 0, 9);
    p_glEndTransformFeedback();
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    expect("transform feedback", "GL_PRIMITIVES_GENERATED", q[0], 3);
    expect("transform feedback", "GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN", q[1], 3);
    expect("transform feedback", "GL_VERTICES_SUBMITTED", q[2], 9);

    /* A statistics query that begins and ends while both count. */
    p_glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[1]);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[0]);
    p_glBeginTransformFeedback(GL_TRIANGLES);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[2]);
    p_glDrawArrays(GL_TRIANGLES, 0, 6);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glEndTransformFeedback();
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
    expect("transform feedback split", "GL_PRIMITIVES_GENERATED", q[0], 4);
    expect("transform feedback split", "GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN", q[1], 4);
    expect("transform feedback split", "GL_VERTICES_SUBMITTED", q[2], 6);

    /* The same with no query ending before the last draw, so the queries
     * begin before the render pass of the draws; a statistics query joins
     * while GL_PRIMITIVES_GENERATED and the written count share the
     * stream's query. */
    p_glFinish();
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, buffer);
    p_glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[1]);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[0]);
    p_glBeginTransformFeedback(GL_TRIANGLES);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[2]);
    p_glDrawArrays(GL_TRIANGLES, 0, 6);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glEndTransformFeedback();
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
    expect("transform feedback join", "GL_PRIMITIVES_GENERATED", q[0], 4);
    expect("transform feedback join", "GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN", q[1], 4);
    expect("transform feedback join", "GL_VERTICES_SUBMITTED", q[2], 9);

    /* GL_PRIMITIVES_GENERATED ends while the written count goes on. */
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, buffer);
    p_glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[1]);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[0]);
    p_glBeginTransformFeedback(GL_TRIANGLES);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glDrawArrays(GL_TRIANGLES, 0, 6);
    p_glEndTransformFeedback();
    p_glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
    expect("generated ends first", "GL_PRIMITIVES_GENERATED", q[0], 1);
    expect("generated ends first", "GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN", q[1], 3);

    /* The written count begins after GL_PRIMITIVES_GENERATED has counted a
     * draw on the stream's query: it counts only the draw after it. */
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, buffer);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[0]);
    p_glBeginTransformFeedback(GL_TRIANGLES);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[1]);
    p_glDrawArrays(GL_TRIANGLES, 0, 6);
    p_glEndTransformFeedback();
    p_glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
    expect("written begins later", "GL_PRIMITIVES_GENERATED", q[0], 3);
    expect("written begins later", "GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN", q[1], 2);
    check_error("transform feedback");
}

/* Queries begun before a dispatch: the compute counter counts the
 * dispatch while a draw counter ends before any draw. */
static void test_dispatch_only(void)
{
    GLuint q[2];

    p_glGenQueries(2, q);
    p_glBeginQuery(GL_COMPUTE_SHADER_INVOCATIONS, q[0]);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[1]);
    dispatch();
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    p_glEndQuery(GL_COMPUTE_SHADER_INVOCATIONS);
    expect("dispatch only", "GL_COMPUTE_SHADER_INVOCATIONS", q[0], 32);
    expect("dispatch only", "GL_VERTICES_SUBMITTED", q[1], 0);
    check_error("dispatch only");
}

/* An occlusion query, the compute counter, GL_PRIMITIVES_GENERATED and the
 * written count begun before a dispatch, then transform feedback draws with
 * the occlusion query ending between them. */
static void test_dispatch_transform_feedback(void)
{
    GLuint q[4], buffer;

    p_glGenQueries(4, q);
    p_glGenBuffers(1, &buffer);
    p_glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, buffer);
    p_glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, 64 * 16, NULL, GL_STREAM_READ);
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, buffer);
    p_glFinish();
    p_glBeginQuery(GL_SAMPLES_PASSED, q[3]);
    p_glBeginQuery(GL_COMPUTE_SHADER_INVOCATIONS, q[0]);
    p_glBeginQuery(GL_PRIMITIVES_GENERATED, q[1]);
    p_glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q[2]);
    dispatch();
    p_glUseProgram(xfb_program);
    p_glBeginTransformFeedback(GL_TRIANGLES);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glEndQuery(GL_SAMPLES_PASSED);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glEndTransformFeedback();
    p_glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
    p_glEndQuery(GL_PRIMITIVES_GENERATED);
    p_glEndQuery(GL_COMPUTE_SHADER_INVOCATIONS);
    p_glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
    expect("dispatch and transform feedback", "GL_COMPUTE_SHADER_INVOCATIONS", q[0], 32);
    expect("dispatch and transform feedback", "GL_PRIMITIVES_GENERATED", q[1], 2);
    expect("dispatch and transform feedback", "GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN", q[2], 2);
    expect_nonzero("dispatch and transform feedback", "GL_SAMPLES_PASSED", q[3]);
    check_error("dispatch and transform feedback");
}

/* A query that begins while another counts and ends before any draw never
 * counts on a Vulkan query: in a query buffer object it is available with
 * the result 0, as 32-bit and as 64-bit values. */
static void test_query_buffer_unstarted(void)
{
    unsigned char data[48], fill[sizeof(data)];
    GLuint q[2], buffer, values[3];
    GLuint64 values64[2];
    unsigned int j;

    memset(fill, 0xaa, sizeof(fill));
    p_glGenQueries(2, q);
    p_glFinish();
    p_glBeginQuery(GL_PRIMITIVES_SUBMITTED, q[0]);
    draw(GL_TRIANGLES, 0, 3);
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[1]);
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    p_glEndQuery(GL_PRIMITIVES_SUBMITTED);
    p_glGenBuffers(1, &buffer);
    p_glBindBuffer(GL_QUERY_BUFFER, buffer);
    p_glBufferData(GL_QUERY_BUFFER, sizeof(fill), fill, GL_STREAM_READ);
    p_glGetQueryObjectuiv(q[1], GL_QUERY_RESULT_AVAILABLE, (GLuint *)(uintptr_t)0);
    p_glGetQueryObjectuiv(q[1], GL_QUERY_RESULT, (GLuint *)(uintptr_t)4);
    p_glGetQueryObjectuiv(q[0], GL_QUERY_RESULT, (GLuint *)(uintptr_t)8);
    p_glGetQueryObjectui64v(q[1], GL_QUERY_RESULT_AVAILABLE, (GLuint64 *)(uintptr_t)16);
    p_glGetQueryObjectui64v(q[1], GL_QUERY_RESULT, (GLuint64 *)(uintptr_t)24);
    p_glFinish();
    p_glGetBufferSubData(GL_QUERY_BUFFER, 0, sizeof(data), data);
    p_glBindBuffer(GL_QUERY_BUFFER, 0);

    memcpy(values, data, sizeof(values));
    memcpy(values64, data + 16, sizeof(values64));
    if (values[0] != 1)
        report("query buffer, unstarted", "the 32-bit availability of GL_VERTICES_SUBMITTED", values[0], "1");
    if (values[1] != 0)
        report("query buffer, unstarted", "the 32-bit GL_VERTICES_SUBMITTED", values[1], "0");
    if (values[2] != 1)
        report("query buffer, unstarted", "GL_PRIMITIVES_SUBMITTED", values[2], "1");
    if (values64[0] != 1)
        report("query buffer, unstarted", "the 64-bit availability of GL_VERTICES_SUBMITTED", values64[0], "1");
    if (values64[1] != 0)
        report("query buffer, unstarted", "the 64-bit GL_VERTICES_SUBMITTED", values64[1], "0");
    for (j = 0; j < sizeof(data); j++)
    {
        if ((j >= 12 && j < 16) || j >= 32)
        {
            if (data[j] != 0xaa)
                report("query buffer, unstarted", "a byte around the results", data[j], "170, as filled");
        }
    }
    check_error("query buffer, unstarted");
}

/* One query active across many short ones: each gives it a start of its
 * own, more than the largest query buffer Zink makes holds (512). */
#define MANY_STARTS 600
static void test_many_starts(void)
{
    GLuint q[2];
    unsigned int i;

    p_glGenQueries(2, q);
    p_glFinish();
    p_glBeginQuery(GL_VERTICES_SUBMITTED, q[0]);
    for (i = 0; i < MANY_STARTS; i++)
    {
        p_glBeginQuery(GL_PRIMITIVES_SUBMITTED, q[1]);
        draw(GL_TRIANGLES, 0, 3);
        p_glEndQuery(GL_PRIMITIVES_SUBMITTED);
    }
    p_glEndQuery(GL_VERTICES_SUBMITTED);
    expect("many starts", "GL_VERTICES_SUBMITTED", q[0], MANY_STARTS * 3);
    expect("many starts", "GL_PRIMITIVES_SUBMITTED", q[1], 1);
    check_error("many starts");
}

/* Results written by the GPU into a query buffer object: 64-bit and 32-bit
 * values and availability, each between bytes that must stay as they were. */
static void test_query_buffer(void)
{
    static const struct { GLenum target; GLuint64 want; } counters[] =
    {
        { GL_VERTICES_SUBMITTED, 9 },
        { GL_PRIMITIVES_SUBMITTED, 3 },
        { GL_CLIPPING_INPUT_PRIMITIVES, 3 },
    };
    unsigned char data[3 * 32], fill[sizeof(data)];
    GLuint q[3], buffer;
    unsigned int i, j;

    memset(fill, 0xaa, sizeof(fill));
    p_glGenQueries(3, q);
    p_glGenBuffers(1, &buffer);
    p_glBindBuffer(GL_QUERY_BUFFER, buffer);
    p_glBufferData(GL_QUERY_BUFFER, sizeof(fill), fill, GL_STREAM_READ);
    for (i = 0; i < 3; i++)
        p_glBeginQuery(counters[i].target, q[i]);
    draw(GL_TRIANGLES, 0, 9);
    /* in reverse, so each query has the one start of the shared Vulkan query
     * and the GPU copies each counter from where it is in that query */
    for (i = 3; i-- > 0;)
        p_glEndQuery(counters[i].target);
    /* Per query 32 bytes: the 64-bit result at 0, the 32-bit result at 12,
     * availability as 32 bits at 20; everything else keeps 0xaa. */
    for (i = 0; i < 3; i++)
    {
        p_glGetQueryObjectui64v(q[i], GL_QUERY_RESULT, (GLuint64 *)(uintptr_t)(i * 32));
        p_glGetQueryObjectuiv(q[i], GL_QUERY_RESULT, (GLuint *)(uintptr_t)(i * 32 + 12));
        p_glGetQueryObjectuiv(q[i], GL_QUERY_RESULT_AVAILABLE, (GLuint *)(uintptr_t)(i * 32 + 20));
    }
    p_glFinish();
    p_glGetBufferSubData(GL_QUERY_BUFFER, 0, sizeof(data), data);
    p_glBindBuffer(GL_QUERY_BUFFER, 0);

    for (i = 0; i < 3; i++)
    {
        unsigned char *base = data + i * 32;
        GLuint64 value64;
        GLuint value32, available;
        char what[64], want[32];

        memcpy(&value64, base, 8);
        memcpy(&value32, base + 12, 4);
        memcpy(&available, base + 20, 4);
        snprintf(want, sizeof(want), "%llu", (unsigned long long)counters[i].want);
        snprintf(what, sizeof(what), "query %u, the 64-bit result", i);
        if (value64 != counters[i].want)
            report("query buffer", what, value64, want);
        snprintf(what, sizeof(what), "query %u, the 32-bit result", i);
        if (value32 != counters[i].want)
            report("query buffer", what, value32, want);
        snprintf(what, sizeof(what), "query %u, availability", i);
        if (available != 1)
            report("query buffer", what, available, "1");
        for (j = 0; j < 32; j++)
        {
            if (j < 8 || (j >= 12 && j < 16) || (j >= 20 && j < 24))
                continue;
            if (base[j] != 0xaa)
            {
                snprintf(what, sizeof(what), "query %u, byte %u around the results", i, j);
                report("query buffer", what, base[j], "170, as filled");
            }
        }
    }
    check_error("query buffer");
}

/* Occlusion queries over many frames, as a game that tests visibility with a
 * few query objects does, with one more query object now and then that is
 * not begun again and is read only at the end. Each query counts a scissor
 * rectangle of the 64x64 target that a triangle covering the target fills;
 * the reused query objects write their results into a query buffer object,
 * which is read once a frame. KosmicKrisp has 32768 occlusion query slots
 * for the device, and a Zink query pool takes up to 512 of them: a query
 * object that is not begun again has to hold only its own query of a pool,
 * and Zink has to use the pool's other queries again, or the 100 queries
 * kept here take every slot and the queries begun after that count
 * nothing. */
#define SLOT_FRAMES 100
#define SLOT_FRAME_QUERIES 1000
#define SLOT_RING 4
static void test_query_slots(void)
{
    GLuint ring[SLOT_RING], kept[SLOT_FRAMES], buffer;
    GLuint64 values[SLOT_FRAME_QUERIES], got;
    unsigned int frame, i, wrong = 0, total = 0;

    p_glGenQueries(SLOT_RING, ring);
    p_glGenQueries(SLOT_FRAMES, kept);
    p_glGenBuffers(1, &buffer);
    p_glBindBuffer(GL_QUERY_BUFFER, buffer);
    p_glBufferData(GL_QUERY_BUFFER, sizeof(values), NULL, GL_STREAM_READ);
    p_glUseProgram(cover_program);
    p_glEnable(GL_SCISSOR_TEST);
    for (frame = 0; frame < SLOT_FRAMES; frame++)
    {
        for (i = 0; i < SLOT_FRAME_QUERIES; i++)
        {
            GLuint q = ring[i % SLOT_RING];

            p_glScissor(0, 0, 1 + i % 64, 64);
            p_glBeginQuery(GL_SAMPLES_PASSED, q);
            p_glDrawArrays(GL_TRIANGLES, 0, 3);
            p_glEndQuery(GL_SAMPLES_PASSED);
            p_glGetQueryObjectui64v(q, GL_QUERY_RESULT, (GLuint64 *)(uintptr_t)(i * sizeof(GLuint64)));
        }
        p_glScissor(0, 0, 1 + frame % 64, 32);
        p_glBeginQuery(GL_SAMPLES_PASSED, kept[frame]);
        p_glDrawArrays(GL_TRIANGLES, 0, 3);
        p_glEndQuery(GL_SAMPLES_PASSED);

        p_glGetBufferSubData(GL_QUERY_BUFFER, 0, sizeof(values), values);
        for (i = 0; i < SLOT_FRAME_QUERIES; i++, total++)
        {
            if (values[i] == 64 * (1 + i % 64))
                continue;
            if (wrong++ < 5)
                printf("glstats: query slots: frame %u, query %u counted %llu samples, expected %u\n",
                       frame, i, (unsigned long long)values[i], 64 * (1 + i % 64));
        }
    }
    p_glDisable(GL_SCISSOR_TEST);
    p_glBindBuffer(GL_QUERY_BUFFER, 0);
    for (frame = 0; frame < SLOT_FRAMES; frame++, total++)
    {
        got = result(kept[frame]);
        if (got == 32 * (1 + frame % 64))
            continue;
        if (wrong++ < 5)
            printf("glstats: query slots: the query kept from frame %u counted %llu samples, expected %u\n",
                   frame, (unsigned long long)got, 32 * (1 + frame % 64));
    }
    if (wrong)
        printf("glstats: query slots: %u of %u queries counted wrong\n", wrong, total);
    failures += wrong;
    check_error("query slots");
}

/* More occlusion queries alive at once than KosmicKrisp has slots for. Each
 * of SPILL_OBJECTS query objects stays active over SPILL_FLUSHES draws with
 * a glFlush after each, so Zink gives it a Vulkan query of its own for every
 * flush, and keeps them all until the object is begun again or deleted; the
 * objects are read only at the end. (Query objects of their own would do the
 * same, but each gets a buffer of its own, and so many exceed the device's
 * maxMemoryAllocationCount.) Once the slots are gone vkCreateQueryPool
 * fails, and Zink has to go on without a Vulkan query for the draws after
 * that, instead of crashing. An occlusion query that missed draws that way
 * reports them visible (SPILL_VISIBLE samples), so that culling and
 * conditional rendering draw rather than hide them. So the objects count
 * all their draws up to the first draw without a Vulkan query, and from the
 * object of that draw on they report SPILL_VISIBLE; the last one also into
 * a query buffer object, and a conditional render on it draws. The case fails when every draw got a Vulkan
 * query: it is for a driver with KosmicKrisp's slot limit. Once the objects
 * are deleted, the slots are free again and new queries count, at the
 * latest SPILL_FRAMES frames later. */
#define SPILL_OBJECTS 8
#define SPILL_FLUSHES 4500
#define SPILL_FRAMES 16
#define SPILL_VISIBLE 0x7fffffff
static void test_query_spill(void)
{
    unsigned int frame, i, j, wrong = 0;
    GLuint spill[SPILL_OBJECTS], ring[SLOT_RING], buffer;
    GLuint64 got, want, written[2];
    unsigned char pixel[4] = { 0 };
    int first_visible = -1;

    p_glGenQueries(SPILL_OBJECTS, spill);
    p_glUseProgram(cover_program);
    p_glEnable(GL_SCISSOR_TEST);
    for (i = 0; i < SPILL_OBJECTS; i++)
    {
        p_glScissor(0, 0, 8 * (i + 1), 64);
        p_glBeginQuery(GL_SAMPLES_PASSED, spill[i]);
        for (j = 0; j < SPILL_FLUSHES; j++)
        {
            p_glDrawArrays(GL_TRIANGLES, 0, 3);
            p_glFlush();
        }
        p_glEndQuery(GL_SAMPLES_PASSED);
    }
    for (i = 0; i < SPILL_OBJECTS; i++)
    {
        got = result(spill[i]);
        want = 8 * (i + 1) * 64 * SPILL_FLUSHES;
        if (got == SPILL_VISIBLE && first_visible < 0)
            first_visible = i;
        else if (got != (first_visible < 0 ? want : SPILL_VISIBLE) && wrong++ < 5)
            printf("glstats: query spill: object %u counted %llu samples, expected %llu\n", i,
                   (unsigned long long)got,
                   first_visible < 0 ? (unsigned long long)want : SPILL_VISIBLE);
    }
    if (first_visible < 0)
    {
        printf("glstats: query spill: every draw got a Vulkan query, the slots never ran out\n");
        wrong++;
    }
    else
        printf("glstats: query spill: the slots ran out in object %d of %u\n", first_visible,
               SPILL_OBJECTS);

    /* the last object reports visible also into a query buffer object, as a
     * 64-bit and a 32-bit value */
    memset(written, 0, sizeof(written));
    p_glGenBuffers(1, &buffer);
    p_glBindBuffer(GL_QUERY_BUFFER, buffer);
    p_glBufferData(GL_QUERY_BUFFER, sizeof(written), written, GL_STREAM_READ);
    p_glGetQueryObjectui64v(spill[SPILL_OBJECTS - 1], GL_QUERY_RESULT, (GLuint64 *)0);
    p_glGetQueryObjectuiv(spill[SPILL_OBJECTS - 1], GL_QUERY_RESULT,
                          (GLuint *)(uintptr_t)sizeof(GLuint64));
    p_glGetBufferSubData(GL_QUERY_BUFFER, 0, sizeof(written), written);
    p_glBindBuffer(GL_QUERY_BUFFER, 0);
    if (first_visible >= 0 && (written[0] != SPILL_VISIBLE || (GLuint)written[1] != SPILL_VISIBLE))
    {
        printf("glstats: query spill: the last object wrote %llu and %u into a query buffer, expected %u\n",
               (unsigned long long)written[0], (GLuint)written[1], SPILL_VISIBLE);
        wrong++;
    }

    /* the last object reports visible, so the conditional render draws */
    p_glDisable(GL_SCISSOR_TEST);
    p_glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
    p_glClear(GL_COLOR_BUFFER_BIT);
    p_glBeginConditionalRender(spill[SPILL_OBJECTS - 1], GL_QUERY_WAIT);
    p_glDrawArrays(GL_TRIANGLES, 0, 3);
    p_glEndConditionalRender();
    p_glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, pixel);
    if (first_visible >= 0 && pixel[0] != 0xff)
    {
        printf("glstats: query spill: a conditional render on a query that missed draws did not draw\n");
        wrong++;
    }
    p_glEnable(GL_SCISSOR_TEST);
    p_glDeleteQueries(SPILL_OBJECTS, spill);

    /* Zink frees the queries of the deleted objects once the batches that
     * could still use them are done and their states are reused, which
     * takes a few flushes: frames of queries, each ending in a glFinish,
     * until all queries of a frame count again; until then they report
     * visible */
    p_glGenQueries(SLOT_RING, ring);
    for (frame = 0; frame < SPILL_FRAMES; frame++)
    {
        unsigned int frame_counted = 0;

        for (i = 0; i < SLOT_FRAME_QUERIES; i++)
        {
            GLuint q = ring[i % SLOT_RING];

            p_glScissor(0, 0, 1 + i % 64, 64);
            p_glBeginQuery(GL_SAMPLES_PASSED, q);
            p_glDrawArrays(GL_TRIANGLES, 0, 3);
            p_glEndQuery(GL_SAMPLES_PASSED);
            got = result(q);
            want = 64 * (1 + i % 64);
            if (got == want)
                frame_counted++;
            else if (got != SPILL_VISIBLE && wrong++ < 5)
                printf("glstats: query spill: after the delete, frame %u, query %u counted %llu samples, expected %llu or %u\n",
                       frame, i, (unsigned long long)got, (unsigned long long)want, SPILL_VISIBLE);
        }
        p_glFinish();
        if (frame_counted == SLOT_FRAME_QUERIES)
            break;
    }
    if (frame == SPILL_FRAMES)
    {
        printf("glstats: query spill: %u frames after the delete, queries still got no Vulkan query\n",
               SPILL_FRAMES);
        wrong++;
    }
    else
        printf("glstats: query spill: frame %u after the delete counted all its queries\n", frame);
    p_glDeleteQueries(SLOT_RING, ring);
    p_glDisable(GL_SCISSOR_TEST);
    if (wrong)
        printf("glstats: query spill: %u queries counted wrong\n", wrong);
    failures += wrong;
    check_error("query spill");
}

static GLuint compile(GLenum type, const char *source)
{
    GLuint shader = p_glCreateShader(type);
    GLint ok = 0;

    p_glShaderSource(shader, 1, &source, NULL);
    p_glCompileShader(shader);
    p_glGetShaderiv(shader, GL_COMPILE_STATUS, &ok);
    return ok ? shader : 0;
}

static GLuint link(GLuint a, GLuint b, GLuint c, int xfb)
{
    static const char *const varyings[] = { "gl_Position" };
    GLuint program = p_glCreateProgram();
    GLint ok = 0;

    p_glAttachShader(program, a);
    if (b)
        p_glAttachShader(program, b);
    if (c)
        p_glAttachShader(program, c);
    if (xfb)
        p_glTransformFeedbackVaryings(program, 1, varyings, GL_INTERLEAVED_ATTRIBS);
    p_glLinkProgram(program);
    p_glGetProgramiv(program, GL_LINK_STATUS, &ok);
    return ok ? program : 0;
}

static int setup(void)
{
    static const char vs[] = "#version 430 core\n"
        "layout(location = 0) in vec2 pos;\n"
        "void main() { gl_Position = vec4(pos, 0.0, 1.0); }\n";
    static const char fs[] = "#version 430 core\n"
        "out vec4 color;\n"
        "void main() { color = vec4(1.0); }\n";
    static const char cs[] = "#version 430 core\n"
        "layout(local_size_x = 8) in;\n"
        "void main() {}\n";
    static const char gs[] = "#version 430 core\n"
        "layout(triangles) in;\n"
        "layout(triangle_strip, max_vertices = 6) out;\n"
        "void main()\n"
        "{\n"
        "    for (int k = 0; k < 2; k++)\n"
        "    {\n"
        "        for (int i = 0; i < 3; i++)\n"
        "        {\n"
        "            gl_Position = gl_in[i].gl_Position;\n"
        "            EmitVertex();\n"
        "        }\n"
        "        EndPrimitive();\n"
        "    }\n"
        "}\n";
    /* one triangle that covers the target */
    static const char cover_vs[] = "#version 430 core\n"
        "void main()\n"
        "{\n"
        "    gl_Position = vec4((gl_VertexID & 1) * 4 - 1, (gl_VertexID >> 1) * 4 - 1, 0.0, 1.0);\n"
        "}\n";
    GLuint vao, vbo, fbo, rb, v, f, c, g, cv;

    p_glGenFramebuffers(1, &fbo);
    p_glBindFramebuffer(GL_FRAMEBUFFER, fbo);
    p_glGenRenderbuffers(1, &rb);
    p_glBindRenderbuffer(GL_RENDERBUFFER, rb);
    p_glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 64, 64);
    p_glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    if (p_glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    {
        printf("glstats: the framebuffer is not complete\n");
        return 0;
    }
    p_glViewport(0, 0, 64, 64);

    p_glGenVertexArrays(1, &vao);
    p_glBindVertexArray(vao);
    p_glGenBuffers(1, &vbo);
    p_glBindBuffer(GL_ARRAY_BUFFER, vbo);
    p_glBufferData(GL_ARRAY_BUFFER, sizeof(vertices), vertices, GL_STATIC_DRAW);
    p_glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, NULL);
    p_glEnableVertexAttribArray(0);

    if (!(v = compile(GL_VERTEX_SHADER, vs)) || !(f = compile(GL_FRAGMENT_SHADER, fs)) ||
        !(c = compile(GL_COMPUTE_SHADER, cs)) || !(g = compile(GL_GEOMETRY_SHADER, gs)) ||
        !(cv = compile(GL_VERTEX_SHADER, cover_vs)) ||
        !(gfx_program = link(v, f, 0, 0)) || !(compute_program = link(c, 0, 0, 0)) ||
        !(geometry_program = link(v, g, f, 0)) || !(xfb_program = link(v, f, 0, 1)) ||
        !(cover_program = link(cv, f, 0, 0)))
    {
        printf("glstats: a shader did not compile or link\n");
        return 0;
    }
    if (p_glGetError())
    {
        printf("glstats: GL error during setup\n");
        return 0;
    }
    return 1;
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
    void *egl;

    if (argc != 2)
    {
        printf("usage: %s <libEGL.1.dylib>\n", argv[0]);
        return FAIL_DLOPEN;
    }
    if (!(egl = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("glstats: cannot load %s: %s\n", argv[1], dlerror());
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
        printf("glstats: %s lacks an EGL 1.5 entry point\n", argv[1]);
        return FAIL_DLOPEN;
    }
    display = get_platform_display(EGL_PLATFORM_SURFACELESS_MESA, NULL, NULL);
    if (!display || !initialize(display, &major, &minor))
    {
        printf("glstats: eglInitialize on the surfaceless platform failed\n");
        return FAIL_INITIALIZE;
    }
    if (!bind_api(EGL_OPENGL_API) || !choose_config(display, config_attribs, &config, 1, &count) ||
        !count || !(context = create_context(display, config, EGL_NO_CONTEXT, context_attribs)) ||
        !make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, context))
    {
        printf("glstats: cannot make an OpenGL 4.6 core context current\n");
        return FAIL_CONTEXT;
    }

#define X(ret, name, args) \
    if (!(p_##name = (ret (*) args)get_proc_address(#name))) \
    { \
        printf("glstats: no %s\n", #name); \
        return FAIL_FUNCTIONS; \
    }
    GL_FUNCS
#undef X
    printf("glstats: GL_RENDERER: %s\n", p_glGetString(GL_RENDERER));
    printf("glstats: GL_VERSION: %s\n", p_glGetString(GL_VERSION));
    if (!setup())
        return FAIL_SETUP;

    test_single();
    test_together();
    test_staggered();
    test_clip();
    test_generated();
    test_compute();
    test_line_loop();
    test_geometry();
    test_transform_feedback();
    test_dispatch_only();
    test_dispatch_transform_feedback();
    test_many_starts();
    test_query_buffer();
    test_query_buffer_unstarted();
    test_query_slots();
    test_query_spill();

    printf("glstats: %u wrong values\n", failures);
    make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    terminate(display);
    return failures > 100 ? 100 : failures;
}
