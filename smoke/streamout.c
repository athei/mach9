/* The Direct3D 11 stream output program bundle-wine.sh builds for each x86 PE
 * architecture and runs under the bundled wine on DXMT, the dxgi tree
 * compatdb.so gives every process by default. Its cases draw through DXMT's
 * emulated stream output, a vertex shader passed to
 * CreateGeometryShaderWithStreamOutput with D3D11_SO_NO_RASTERIZED_STREAM,
 * and check what each draw wrote:
 *
 *   bounds           a target of 256 bytes bound at offset 64 (room for 12
 *                    vertices of 16 bytes), 64 vertices drawn: the 12 that fit
 *                    are written, nothing before the offset, nothing into 8
 *                    buffers created after the target
 *   append           the append offset 0xffffffff: nothing written, or the 4
 *                    vertices at offset 0
 *   unbind           a draw, then slot 0 unbound and a draw with other
 *                    vertices: only the first draw's vertices
 *   no-target        a draw with no target ever bound: nothing written
 *   pipeline-switch  a draw through a geometry shader without stream output,
 *                    then the stream output shader drawn in the same render
 *                    encoder without SOSetTargets: its 4 vertices are written
 *   packed           the declaration {ATTRIB3 .xy, ATTRIB4} of a shader whose
 *                    signature packs ATTRIB4 into .z of ATTRIB3's register:
 *                    the third word of each vertex is ATTRIB4
 *   clear-state      a draw, ClearState, everything but the target bound again
 *                    and a draw with other vertices: the second draw writes
 *                    nothing
 *   swap-state       a context state binds target A, the default state target
 *                    B; after swapping back, a draw of 8 other vertices writes
 *                    them into A, at offset 0 or after the first draw's, and
 *                    nothing into B
 *   start-component  a draw with {POSITION .xyzw}, then a new geometry shader
 *                    with {POSITION .zw} drawn into another target: that
 *                    target holds .z and .w of each vertex and nothing else
 *   signature-blob   a shader created from an output signature blob streams
 *                    out; without a declaration such a blob is refused, and so
 *                    is one whose header gives a size past the length passed
 *
 * Every case runs on a device of its own, and every buffer starts filled with
 * a pattern. A case reads its targets back between draws that write the same
 * range, which waits for the GPU, so no two draws of a case run at the same
 * time; DXMT does not order such draws.
 *
 * A store outside a buffer may land where no readback sees it. Metal shader
 * validation (MTL_SHADER_VALIDATION=1) reports such a store, or a load of an
 * unset table, in the command buffer's logs, which DXMT prints as "err:
 * Frame <n>: Invalid store ..." lines; bundle-wine.sh runs the program with
 * validation on and fails on those lines. It also turns DXMT's shader cache
 * off (DXMT_SHADER_CACHE=0), so every shader is compiled by the DXMT under
 * test.
 *
 * The cases run in order in one process, or just those named on the command
 * line. Each prints its name first. A case whose result is wrong prints why
 * and ends the process with 1, a call that fails with 2; 0 means every case
 * passed.
 *
 * The shaders are the vs_code of test_stream_output_vs (POSITION, COLOR0 and
 * COLOR1 passed through), of test_stream_output (SV_Position and ATTRIB1 to
 * ATTRIB4, ATTRIB3 and ATTRIB4 packed into one register) and the geometry
 * shader of test_stream_output in Wine's dlls/d3d11/tests/d3d11.c. */

#define COBJMACROS
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>
#include <initguid.h>
#include <d3d11_1.h>

#if defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

/* struct vertex { float4 position : POSITION; float4 color0 : COLOR0;
 *                 float4 color1 : COLOR1; };
 * vertex main(in vertex i) { return i; } */
static const DWORD vs_code[] = {
    0x43425844, 0xa67e993e, 0x1632c139, 0x02a7725f, 0xfb0221cd, 0x00000001, 0x00000194, 0x00000003,
    0x0000002c, 0x00000094, 0x000000fc, 0x4e475349, 0x00000060, 0x00000003, 0x00000008, 0x00000050,
    0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000f0f, 0x00000059, 0x00000000, 0x00000000,
    0x00000003, 0x00000001, 0x00000f0f, 0x00000059, 0x00000001, 0x00000000, 0x00000003, 0x00000002,
    0x00000f0f, 0x49534f50, 0x4e4f4954, 0x4c4f4300, 0xab00524f, 0x4e47534f, 0x00000060, 0x00000003,
    0x00000008, 0x00000050, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x0000000f, 0x00000059,
    0x00000000, 0x00000000, 0x00000003, 0x00000001, 0x0000000f, 0x00000059, 0x00000001, 0x00000000,
    0x00000003, 0x00000002, 0x0000000f, 0x49534f50, 0x4e4f4954, 0x4c4f4300, 0xab00524f, 0x52444853,
    0x00000090, 0x00010040, 0x00000024, 0x0300005f, 0x001010f2, 0x00000000, 0x0300005f, 0x001010f2,
    0x00000001, 0x0300005f, 0x001010f2, 0x00000002, 0x03000065, 0x001020f2, 0x00000000, 0x03000065,
    0x001020f2, 0x00000001, 0x03000065, 0x001020f2, 0x00000002, 0x05000036, 0x001020f2, 0x00000000,
    0x00101e46, 0x00000000, 0x05000036, 0x001020f2, 0x00000001, 0x00101e46, 0x00000001, 0x05000036,
    0x001020f2, 0x00000002, 0x00101e46, 0x00000002, 0x0100003e,
};

/* struct data { float4 position : SV_Position; float4 attrib1 : ATTRIB1;
 *               float3 attrib2 : attrib2; float2 attrib3 : ATTriB3;
 *               float attrib4 : ATTRIB4; };
 * void main(in data i, out data o) { o = i; } */
static const DWORD packed_vs_code[] = {
    0x43425844, 0x3f5b621f, 0x8f390786, 0x7235c8d6, 0xc1181ad3, 0x00000001, 0x00000278, 0x00000003,
    0x0000002c, 0x000000d8, 0x00000184, 0x4e475349, 0x000000a4, 0x00000005, 0x00000008, 0x00000080,
    0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000f0f, 0x0000008c, 0x00000001, 0x00000000,
    0x00000003, 0x00000001, 0x00000f0f, 0x00000093, 0x00000002, 0x00000000, 0x00000003, 0x00000002,
    0x00000707, 0x0000009a, 0x00000003, 0x00000000, 0x00000003, 0x00000003, 0x00000303, 0x0000008c,
    0x00000004, 0x00000000, 0x00000003, 0x00000004, 0x00000101, 0x505f5653, 0x7469736f, 0x006e6f69,
    0x52545441, 0x61004249, 0x69727474, 0x54410062, 0x42697254, 0xababab00, 0x4e47534f, 0x000000a4,
    0x00000005, 0x00000008, 0x00000080, 0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x0000000f,
    0x0000008c, 0x00000001, 0x00000000, 0x00000003, 0x00000001, 0x0000000f, 0x00000093, 0x00000002,
    0x00000000, 0x00000003, 0x00000002, 0x00000807, 0x0000009a, 0x00000003, 0x00000000, 0x00000003,
    0x00000003, 0x00000c03, 0x0000008c, 0x00000004, 0x00000000, 0x00000003, 0x00000003, 0x00000b04,
    0x505f5653, 0x7469736f, 0x006e6f69, 0x52545441, 0x61004249, 0x69727474, 0x54410062, 0x42697254,
    0xababab00, 0x52444853, 0x000000ec, 0x00010040, 0x0000003b, 0x0300005f, 0x001010f2, 0x00000000,
    0x0300005f, 0x001010f2, 0x00000001, 0x0300005f, 0x00101072, 0x00000002, 0x0300005f, 0x00101032,
    0x00000003, 0x0300005f, 0x00101012, 0x00000004, 0x04000067, 0x001020f2, 0x00000000, 0x00000001,
    0x03000065, 0x001020f2, 0x00000001, 0x03000065, 0x00102072, 0x00000002, 0x03000065, 0x00102032,
    0x00000003, 0x03000065, 0x00102042, 0x00000003, 0x05000036, 0x001020f2, 0x00000000, 0x00101e46,
    0x00000000, 0x05000036, 0x001020f2, 0x00000001, 0x00101e46, 0x00000001, 0x05000036, 0x00102072,
    0x00000002, 0x00101246, 0x00000002, 0x05000036, 0x00102032, 0x00000003, 0x00101046, 0x00000003,
    0x05000036, 0x00102042, 0x00000003, 0x0010100a, 0x00000004, 0x0100003e,
};

/* A geometry shader without stream output, which DXMT runs as a geometry
 * pipeline and not as a pass-through:
 * struct point_data { float4 pos : SV_POSITION; };
 * [maxvertexcount(3)]
 * void main(triangle point_data vin[3], inout TriangleStream<point_data> vout)
 * { for (uint i = 0; i < 3; ++i) vout.Append(vin[i]); } */
static const DWORD triangle_gs_code[] = {
    0x43425844, 0x8e49d18d, 0x6d08d6e5, 0xb7015628, 0xf9351fdd, 0x00000001, 0x00000164, 0x00000003,
    0x0000002c, 0x00000060, 0x00000094, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x00000f0f, 0x505f5653, 0x5449534f, 0x004e4f49,
    0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000001, 0x00000003,
    0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49, 0x52444853, 0x000000c8, 0x00020040,
    0x00000032, 0x05000061, 0x002010f2, 0x00000003, 0x00000000, 0x00000001, 0x02000068, 0x00000001,
    0x0100185d, 0x0100285c, 0x04000067, 0x001020f2, 0x00000000, 0x00000001, 0x0200005e, 0x00000003,
    0x05000036, 0x00100012, 0x00000000, 0x00004001, 0x00000000, 0x01000030, 0x07000050, 0x00100022,
    0x00000000, 0x0010000a, 0x00000000, 0x00004001, 0x00000003, 0x03040003, 0x0010001a, 0x00000000,
    0x07000036, 0x001020f2, 0x00000000, 0x00a01e46, 0x0010000a, 0x00000000, 0x00000000, 0x01000013,
    0x0700001e, 0x00100012, 0x00000000, 0x0010000a, 0x00000000, 0x00004001, 0x00000001, 0x01000016,
    0x0100003e,
};

#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))
#define VERTEX_COUNT 64
#define VERTEX_STRIDE 48
#define TARGET_SIZE 256
#define TARGET_WORDS (TARGET_SIZE / 4)
#define NEIGHBOURS 8
#define TARGET_FILL 0xcdcdcdcdu
#define NEIGHBOUR_FILL 0xababababu

static const char *current = "startup";
static PFN_D3D11_CREATE_DEVICE create_device;

/* What every case starts with (setup): a device, a render target, as in
 * Wine's test context, the input layout and shaders of vs_code with
 * {POSITION .xyzw} streamed out with a stride of 16, the vertex buffers of
 * sets 0 and 1, and one target. */
static ID3D11Device *device;
static ID3D11DeviceContext *context;
static ID3D11Texture2D *rt;
static ID3D11RenderTargetView *rtv;
static ID3D11InputLayout *layout;
static ID3D11VertexShader *vs;
static ID3D11GeometryShader *gs;
static ID3D11Buffer *vb0, *vb1, *target;

static void say(const char *fmt, ...)
{
    va_list args;

    printf("wine-build smoke test: %s ", ARCH);
    va_start(args, fmt);
    vprintf(fmt, args);
    va_end(args);
    printf("\n");
    fflush(stdout);
}

/* Print why the current case failed and exit with <status>. */
static void fail(int status, const char *fmt, ...)
{
    va_list args;

    printf("wine-build smoke test: %s %s failed: ", ARCH, current);
    va_start(args, fmt);
    vprintf(fmt, args);
    va_end(args);
    printf("\n");
    fflush(stdout);
    exit(status);
}

static void check_hr(HRESULT hr, const char *call)
{
    if (FAILED(hr)) fail(2, "%s failed, %#lx", call, hr);
}

/* Component c of vertex v of vertex set s */
static float position(unsigned s, unsigned v, unsigned c)
{
    return 1000.0f * (float)s + (float)v + 0.25f * (float)c + 1.0f;
}

static ID3D11Buffer *create_buffer(UINT size, uint32_t fill)
{
    static uint32_t data[TARGET_WORDS];
    D3D11_BUFFER_DESC desc = { size, D3D11_USAGE_DEFAULT, D3D11_BIND_STREAM_OUTPUT, 0, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { data, 0, 0 };
    ID3D11Buffer *buffer;
    unsigned i;

    for (i = 0; i < size / 4; i++) data[i] = fill;
    check_hr(ID3D11Device_CreateBuffer(device, &desc, &init, &buffer), "CreateBuffer");
    return buffer;
}

/* Vertex buffer of VERTEX_COUNT vertices of VERTEX_STRIDE bytes: POSITION of
 * set s, COLOR0 and COLOR1 zero */
static ID3D11Buffer *create_vertices(unsigned s)
{
    static float vertices[VERTEX_COUNT][VERTEX_STRIDE / 4];
    D3D11_BUFFER_DESC desc = { sizeof(vertices), D3D11_USAGE_DEFAULT, D3D11_BIND_VERTEX_BUFFER, 0, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { vertices, 0, 0 };
    ID3D11Buffer *buffer;
    unsigned v, c;

    for (v = 0; v < VERTEX_COUNT; v++)
        for (c = 0; c < VERTEX_STRIDE / 4; c++) vertices[v][c] = c < 4 ? position(s, v, c) : 0.0f;
    check_hr(ID3D11Device_CreateBuffer(device, &desc, &init, &buffer), "CreateBuffer");
    return buffer;
}

/* Copy the TARGET_SIZE bytes of <buffer> into <out>. The Map waits for the
 * GPU to finish every draw before it. */
static void read_buffer(ID3D11Buffer *buffer, uint32_t *out)
{
    D3D11_BUFFER_DESC desc = { TARGET_SIZE, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ, 0, 0 };
    D3D11_MAPPED_SUBRESOURCE map;
    ID3D11Buffer *staging;

    check_hr(ID3D11Device_CreateBuffer(device, &desc, NULL, &staging), "CreateBuffer");
    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging, (ID3D11Resource *)buffer);
    check_hr(ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &map), "Map");
    memcpy(out, map.pData, TARGET_SIZE);
    ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, 0);
    ID3D11Buffer_Release(staging);
}

/* Words <from> to <to> - 1 of <data> that differ from <fill> */
static unsigned count_changed(const uint32_t *data, unsigned from, unsigned to, uint32_t fill)
{
    unsigned changed = 0, i;

    for (i = from; i < to; i++) changed += data[i] != fill;
    return changed;
}

/* Whether the 16 bytes at word w are vertex v of set s */
static int is_vertex(const uint32_t *data, unsigned w, unsigned s, unsigned v)
{
    const float *p = (const float *)&data[w];
    unsigned c;

    for (c = 0; c < 4; c++)
        if (p[c] != position(s, v, c)) return 0;
    return 1;
}

/* Whether slots <first> to <first> + <n> - 1 of 16 bytes hold vertices 0 to
 * <n> - 1 of set s */
static int has_vertices(const uint32_t *data, unsigned first, unsigned n, unsigned s)
{
    unsigned v;

    for (v = 0; v < n; v++)
        if (!is_vertex(data, (first + v) * 4, s, v)) return 0;
    return 1;
}

/* Whether any slot of 16 bytes holds a vertex of set s */
static int has_set(const uint32_t *data, unsigned s)
{
    unsigned w, v;

    for (w = 0; w < TARGET_WORDS; w += 4)
        for (v = 0; v < VERTEX_COUNT; v++)
            if (is_vertex(data, w, s, v)) return 1;
    return 0;
}

static void bind_target(ID3D11Buffer *buffer, UINT offset)
{
    ID3D11DeviceContext_SOSetTargets(context, 1, &buffer, &offset);
}

/* Bind everything a draw of setup's shaders needs except the target */
static void bind_pipeline(ID3D11Buffer *vb)
{
    D3D11_VIEWPORT viewport = { 0.0f, 0.0f, 64.0f, 64.0f, 0.0f, 1.0f };
    UINT stride = VERTEX_STRIDE, offset = 0;

    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &rtv, NULL);
    ID3D11DeviceContext_RSSetViewports(context, 1, &viewport);
    ID3D11DeviceContext_IASetInputLayout(context, layout);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &stride, &offset);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
    ID3D11DeviceContext_VSSetShader(context, vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, gs, NULL, 0);
}

static void setup(void)
{
    static const D3D11_INPUT_ELEMENT_DESC layout_desc[] = {
        { "POSITION", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 16, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "COLOR", 1, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 32, D3D11_INPUT_PER_VERTEX_DATA, 0 },
    };
    static const D3D11_SO_DECLARATION_ENTRY so_decl[] = { { 0, "POSITION", 0, 0, 4, 0 } };
    static const D3D_FEATURE_LEVEL level = D3D_FEATURE_LEVEL_11_0;
    D3D11_TEXTURE2D_DESC rt_desc = { 64, 64, 1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT,
                                     D3D11_BIND_RENDER_TARGET, 0, 0 };
    UINT stride = 16;

    check_hr(create_device(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, &level, 1, D3D11_SDK_VERSION, &device, NULL,
                           &context), "D3D11CreateDevice");
    check_hr(ID3D11Device_CreateTexture2D(device, &rt_desc, NULL, &rt), "CreateTexture2D");
    check_hr(ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)rt, NULL, &rtv),
             "CreateRenderTargetView");
    check_hr(ID3D11Device_CreateInputLayout(device, layout_desc, 3, vs_code, sizeof(vs_code), &layout),
             "CreateInputLayout");
    check_hr(ID3D11Device_CreateVertexShader(device, vs_code, sizeof(vs_code), NULL, &vs), "CreateVertexShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, vs_code, sizeof(vs_code), so_decl, 1, &stride,
                                                               1, D3D11_SO_NO_RASTERIZED_STREAM, NULL, &gs),
             "CreateGeometryShaderWithStreamOutput");
    vb0 = create_vertices(0);
    vb1 = create_vertices(1);
    target = create_buffer(TARGET_SIZE, TARGET_FILL);
}

static void teardown(void)
{
    ID3D11DeviceContext_ClearState(context);
    ID3D11DeviceContext_Flush(context);
    ID3D11Buffer_Release(target);
    ID3D11Buffer_Release(vb1);
    ID3D11Buffer_Release(vb0);
    ID3D11GeometryShader_Release(gs);
    ID3D11VertexShader_Release(vs);
    ID3D11InputLayout_Release(layout);
    ID3D11RenderTargetView_Release(rtv);
    ID3D11Texture2D_Release(rt);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
}

static void bounds_case(void)
{
    static uint32_t data[TARGET_WORDS], neighbour_data[TARGET_WORDS];
    ID3D11Buffer *neighbours[NEIGHBOURS];
    unsigned changed = 0, i;

    for (i = 0; i < NEIGHBOURS; i++) neighbours[i] = create_buffer(TARGET_SIZE, NEIGHBOUR_FILL);
    bind_pipeline(vb0);
    bind_target(target, 64);
    ID3D11DeviceContext_Draw(context, VERTEX_COUNT, 0);
    read_buffer(target, data);
    for (i = 0; i < NEIGHBOURS; i++)
    {
        read_buffer(neighbours[i], neighbour_data);
        changed += count_changed(neighbour_data, 0, TARGET_WORDS, NEIGHBOUR_FILL);
        ID3D11Buffer_Release(neighbours[i]);
    }
    if (count_changed(data, 0, 16, TARGET_FILL) || !has_vertices(data, 4, 12, 0) || changed)
        fail(1, "words before the offset changed %u (want 0), the 12 vertices that fit %s, neighbour words "
             "changed %u (want 0)", count_changed(data, 0, 16, TARGET_FILL),
             has_vertices(data, 4, 12, 0) ? "written" : "not written", changed);
}

/* DXMT does not track the filled size the offset 0xffffffff appends at, and
 * writes nothing; a D3D11 that tracks it writes the 4 vertices at offset 0 of
 * this buffer, which was never streamed into. */
static void append_case(void)
{
    static uint32_t data[TARGET_WORDS];
    unsigned changed;

    bind_pipeline(vb0);
    bind_target(target, 0xffffffff);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    changed = count_changed(data, 0, TARGET_WORDS, TARGET_FILL);
    if (changed && !(changed == 16 && has_vertices(data, 0, 4, 0)))
        fail(1, "words changed %u (want 0, or 16 that are the 4 vertices)", changed);
}

static void unbind_case(void)
{
    static uint32_t data[TARGET_WORDS];
    ID3D11Buffer *null_buffer = NULL;
    UINT stride = VERTEX_STRIDE, offset = 0, zero = 0;

    bind_pipeline(vb0);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    ID3D11DeviceContext_SOSetTargets(context, 1, &null_buffer, &zero);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb1, &stride, &offset);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    if (!has_vertices(data, 0, 4, 0) || has_set(data, 1) || count_changed(data, 16, TARGET_WORDS, TARGET_FILL))
        fail(1, "the first draw's vertices %s, the second draw's %s (want none), words after them changed %u "
             "(want 0)", has_vertices(data, 0, 4, 0) ? "kept" : "not kept",
             has_set(data, 1) ? "written" : "not written", count_changed(data, 16, TARGET_WORDS, TARGET_FILL));
}

/* On a new device, so the draw starts a render encoder that never had a
 * target bound */
static void no_target_case(void)
{
    static uint32_t data[TARGET_WORDS];

    bind_pipeline(vb0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    if (count_changed(data, 0, TARGET_WORDS, TARGET_FILL))
        fail(1, "words changed %u (want 0)", count_changed(data, 0, TARGET_WORDS, TARGET_FILL));
}

static void pipeline_switch_case(void)
{
    static uint32_t data[TARGET_WORDS];
    ID3D11GeometryShader *triangle_gs;

    check_hr(ID3D11Device_CreateGeometryShader(device, triangle_gs_code, sizeof(triangle_gs_code), NULL,
                                               &triangle_gs), "CreateGeometryShader");
    bind_pipeline(vb0);
    ID3D11DeviceContext_GSSetShader(context, triangle_gs, NULL, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 3, 0);
    ID3D11DeviceContext_GSSetShader(context, gs, NULL, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    ID3D11GeometryShader_Release(triangle_gs);
    if (!has_vertices(data, 0, 4, 0) || count_changed(data, 16, TARGET_WORDS, TARGET_FILL))
        fail(1, "the 4 vertices %s, words after them changed %u (want 0)",
             has_vertices(data, 0, 4, 0) ? "written" : "not written",
             count_changed(data, 16, TARGET_WORDS, TARGET_FILL));
}

static void packed_case(void)
{
    static const D3D11_INPUT_ELEMENT_DESC desc[] = {
        { "SV_Position", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "ATTRIB", 1, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 16, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "ATTRIB", 2, DXGI_FORMAT_R32G32B32_FLOAT, 0, 32, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "ATTRIB", 3, DXGI_FORMAT_R32G32_FLOAT, 0, 44, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "ATTRIB", 4, DXGI_FORMAT_R32_FLOAT, 0, 52, D3D11_INPUT_PER_VERTEX_DATA, 0 },
    };
    static const D3D11_SO_DECLARATION_ENTRY so_decl[] = {
        { 0, "ATTRIB", 3, 0, 2, 0 },
        { 0, "ATTRIB", 4, 0, 1, 0 },
    };
    static float vertices[4][14];
    static uint32_t data[TARGET_WORDS];
    D3D11_BUFFER_DESC vb_desc = { sizeof(vertices), D3D11_USAGE_DEFAULT, D3D11_BIND_VERTEX_BUFFER, 0, 0, 0 };
    D3D11_SUBRESOURCE_DATA vb_init = { vertices, 0, 0 };
    UINT so_stride = 12, vb_stride = sizeof(vertices[0]), vb_offset = 0;
    ID3D11InputLayout *packed_layout;
    ID3D11VertexShader *packed_vs;
    ID3D11GeometryShader *packed_gs;
    unsigned correct = 0, v, c;
    ID3D11Buffer *vb;

    for (v = 0; v < 4; v++)
    {
        for (c = 0; c < 11; c++) vertices[v][c] = 0.0f;
        vertices[v][11] = 100.0f + v; /* ATTRIB3.x */
        vertices[v][12] = 200.0f + v; /* ATTRIB3.y */
        vertices[v][13] = 300.0f + v; /* ATTRIB4 */
    }
    check_hr(ID3D11Device_CreateBuffer(device, &vb_desc, &vb_init, &vb), "CreateBuffer");
    check_hr(ID3D11Device_CreateInputLayout(device, desc, 5, packed_vs_code, sizeof(packed_vs_code), &packed_layout),
             "CreateInputLayout");
    check_hr(ID3D11Device_CreateVertexShader(device, packed_vs_code, sizeof(packed_vs_code), NULL, &packed_vs),
             "CreateVertexShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, packed_vs_code, sizeof(packed_vs_code),
                                                               so_decl, 2, &so_stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &packed_gs),
             "CreateGeometryShaderWithStreamOutput");
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, packed_layout);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &vb_stride, &vb_offset);
    ID3D11DeviceContext_VSSetShader(context, packed_vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, packed_gs, NULL, 0);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    for (v = 0; v < 4; v++)
    {
        const float *p = (const float *)&data[v * 3];
        correct += p[0] == 100.0f + v && p[1] == 200.0f + v && p[2] == 300.0f + v;
    }
    ID3D11DeviceContext_ClearState(context);
    ID3D11GeometryShader_Release(packed_gs);
    ID3D11VertexShader_Release(packed_vs);
    ID3D11InputLayout_Release(packed_layout);
    ID3D11Buffer_Release(vb);
    if (correct != 4) fail(1, "vertices with ATTRIB3.xy and ATTRIB4 %u/4", correct);
}

static void clear_state_case(void)
{
    static uint32_t data[TARGET_WORDS];

    bind_pipeline(vb0);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    ID3D11DeviceContext_ClearState(context);
    bind_pipeline(vb1);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    if (!has_vertices(data, 0, 4, 0) || has_set(data, 1))
        fail(1, "the first draw's vertices %s, the second draw's %s (want none)",
             has_vertices(data, 0, 4, 0) ? "kept" : "not kept", has_set(data, 1) ? "written" : "not written");
}

/* The last draw goes into A at offset 0 when the target's filled size does not
 * carry over the swaps, as in DXMT, which does not track it, and after the
 * first draw's 4 vertices when it does. */
static void swap_state_case(void)
{
    static const D3D_FEATURE_LEVEL level = D3D_FEATURE_LEVEL_11_0;
    static uint32_t data_a[TARGET_WORDS], data_b[TARGET_WORDS];
    ID3DDeviceContextState *state_a, *state_default;
    UINT stride = VERTEX_STRIDE, offset = 0;
    ID3D11DeviceContext1 *context1;
    ID3D11Device1 *device1;
    ID3D11Buffer *target_b;
    int in_a;

    check_hr(ID3D11Device_QueryInterface(device, &IID_ID3D11Device1, (void **)&device1), "QueryInterface");
    check_hr(ID3D11DeviceContext_QueryInterface(context, &IID_ID3D11DeviceContext1, (void **)&context1),
             "QueryInterface");
    check_hr(ID3D11Device1_CreateDeviceContextState(device1, 0, &level, 1, D3D11_SDK_VERSION, &IID_ID3D11Device,
                                                    NULL, &state_a), "CreateDeviceContextState");
    target_b = create_buffer(TARGET_SIZE, TARGET_FILL);

    ID3D11DeviceContext1_SwapDeviceContextState(context1, state_a, &state_default);
    bind_pipeline(vb0);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data_a);

    ID3D11DeviceContext1_SwapDeviceContextState(context1, state_default, NULL);
    bind_pipeline(vb0);
    bind_target(target_b, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target_b, data_b);

    ID3D11DeviceContext1_SwapDeviceContextState(context1, state_a, NULL);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb1, &stride, &offset);
    ID3D11DeviceContext_Draw(context, 8, 0);
    ID3D11DeviceContext_ClearState(context);
    ID3D11DeviceContext1_SwapDeviceContextState(context1, state_default, NULL);

    read_buffer(target, data_a);
    read_buffer(target_b, data_b);
    ID3D11Buffer_Release(target_b);
    ID3DDeviceContextState_Release(state_default);
    ID3DDeviceContextState_Release(state_a);
    ID3D11DeviceContext1_Release(context1);
    ID3D11Device1_Release(device1);
    in_a = has_vertices(data_a, 0, 8, 1) || (has_vertices(data_a, 0, 4, 0) && has_vertices(data_a, 4, 8, 1));
    if (!in_a || !has_vertices(data_b, 0, 4, 0) || has_set(data_b, 1))
        fail(1, "the last draw's 8 vertices in A %s, B's own vertices %s, the last draw's in B %s (want none)",
             in_a ? "written" : "not written", has_vertices(data_b, 0, 4, 0) ? "kept" : "not kept",
             has_set(data_b, 1) ? "written" : "not written");
}

/* A declaration that starts at a component other than .x, after a draw with
 * another declaration of the same vertex shader. Without strides, the stride
 * is the 8 bytes the declaration packs. */
static void start_component_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY zw_decl[] = { { 0, "POSITION", 0, 2, 2, 0 } };
    static uint32_t data[TARGET_WORDS];
    ID3D11GeometryShader *zw_gs;
    ID3D11Buffer *target_zw;
    unsigned correct = 0, v;

    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, vs_code, sizeof(vs_code), zw_decl, 1, NULL,
                                                               0, D3D11_SO_NO_RASTERIZED_STREAM, NULL, &zw_gs),
             "CreateGeometryShaderWithStreamOutput");
    target_zw = create_buffer(TARGET_SIZE, TARGET_FILL);
    bind_pipeline(vb0);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    ID3D11DeviceContext_GSSetShader(context, zw_gs, NULL, 0);
    bind_target(target_zw, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target_zw, data);
    ID3D11DeviceContext_ClearState(context);
    ID3D11Buffer_Release(target_zw);
    ID3D11GeometryShader_Release(zw_gs);
    for (v = 0; v < 4; v++)
    {
        const float *p = (const float *)&data[v * 2];
        correct += p[0] == position(0, v, 2) && p[1] == position(0, v, 3);
    }
    if (correct != 4 || count_changed(data, 0, TARGET_WORDS, TARGET_FILL) != 8)
        fail(1, "vertices with .z and .w %u/4, words changed %u (want 8)", correct,
             count_changed(data, 0, TARGET_WORDS, TARGET_FILL));
}

/* The bytecode is the output signature of vs_code alone, a DXBC container
 * with only its OSGN chunk, as D3DGetOutputSignatureBlob returns it (built
 * here with a zero checksum, which DXMT does not check). */
static void signature_blob_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "POSITION", 0, 0, 4, 0 } };
    static uint32_t blob[64];
    static uint32_t data[TARGET_WORDS];
    const uint32_t *dwords = (const uint32_t *)vs_code;
    ID3D11GeometryShader *sig_gs = NULL, *no_decl_gs = NULL, *oversized_gs = NULL;
    HRESULT hr, hr_no_decl, hr_oversized;
    uint32_t osgn_size = 0, size, i;
    const char *osgn = NULL;
    UINT stride = 16;
    int written = 0;

    for (i = 0; i < dwords[7]; i++)
    {
        const char *chunk = (const char *)vs_code + dwords[8 + i];
        if (!memcmp(chunk, "OSGN", 4))
        {
            osgn = chunk;
            osgn_size = 8 + *(const uint32_t *)(chunk + 4);
        }
    }
    size = 36 + osgn_size;
    if (!osgn || size > sizeof(blob)) fail(2, "no output signature in vs_code");
    memcpy(blob, "DXBC", 4);
    blob[5] = 1;
    blob[6] = size;
    blob[7] = 1;
    blob[8] = 36;
    memcpy(&blob[9], osgn, osgn_size);

    hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(device, blob, size, decl, 1, &stride, 1,
                                                           D3D11_SO_NO_RASTERIZED_STREAM, NULL, &sig_gs);
    hr_no_decl = ID3D11Device_CreateGeometryShaderWithStreamOutput(device, blob, size, NULL, 0, NULL, 0,
                                                                   D3D11_SO_NO_RASTERIZED_STREAM, NULL, &no_decl_gs);
    blob[6] = 0x200;
    hr_oversized = ID3D11Device_CreateGeometryShaderWithStreamOutput(device, blob, size, decl, 1, &stride, 1,
                                                                     D3D11_SO_NO_RASTERIZED_STREAM, NULL,
                                                                     &oversized_gs);
    if (SUCCEEDED(hr))
    {
        bind_pipeline(vb0);
        ID3D11DeviceContext_GSSetShader(context, sig_gs, NULL, 0);
        bind_target(target, 0);
        ID3D11DeviceContext_Draw(context, 4, 0);
        read_buffer(target, data);
        written = has_vertices(data, 0, 4, 0);
        ID3D11DeviceContext_ClearState(context);
        ID3D11GeometryShader_Release(sig_gs);
    }
    if (no_decl_gs) ID3D11GeometryShader_Release(no_decl_gs);
    if (oversized_gs) ID3D11GeometryShader_Release(oversized_gs);
    if (hr != S_OK || !written || hr_no_decl != E_INVALIDARG || no_decl_gs || hr_oversized != E_INVALIDARG
        || oversized_gs)
        fail(1, "with a declaration %#lx (want 0), vertices %s; without one %#lx (want 0x80070057), shader %s "
             "(want none); header size past the length %#lx (want 0x80070057), shader %s (want none)", hr,
             written ? "written" : "not written", hr_no_decl, no_decl_gs ? "returned" : "none", hr_oversized,
             oversized_gs ? "returned" : "none");
}

static const struct
{
    const char *name;
    void (*run)(void);
} cases[] = {
    { "bounds", bounds_case },
    { "append", append_case },
    { "unbind", unbind_case },
    { "no-target", no_target_case },
    { "pipeline-switch", pipeline_switch_case },
    { "packed", packed_case },
    { "clear-state", clear_state_case },
    { "swap-state", swap_state_case },
    { "start-component", start_component_case },
    { "signature-blob", signature_blob_case },
};

static void run_case(unsigned i)
{
    current = cases[i].name;
    say("case %s", current);
    setup();
    cases[i].run();
    teardown();
}

int main(int argc, char **argv)
{
    unsigned i;
    HMODULE d3d11;
    int a;

    if (!(d3d11 = LoadLibraryA("d3d11.dll"))) fail(2, "cannot load d3d11.dll, error %lu", GetLastError());
    if (!(create_device = (PFN_D3D11_CREATE_DEVICE)(void *)GetProcAddress(d3d11, "D3D11CreateDevice")))
        fail(2, "d3d11.dll has no D3D11CreateDevice");
    if (argc == 1)
        for (i = 0; i < ARRAY_SIZE(cases); i++) run_case(i);
    for (a = 1; a < argc; a++)
    {
        for (i = 0; i < ARRAY_SIZE(cases); i++)
            if (!strcmp(argv[a], cases[i].name)) break;
        if (i == ARRAY_SIZE(cases)) fail(2, "no case %s", argv[a]);
        run_case(i);
    }
    say("stream output cases passed");
    return 0;
}
