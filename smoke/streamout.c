/* The Direct3D 11 stream output program bundle-wine.sh builds for each x86 PE
 * architecture and runs under the bundled wine on DXMT, the dxgi tree
 * compatdb.so gives every process by default. Its cases draw through DXMT's
 * emulated stream output, a vertex shader passed to
 * CreateGeometryShaderWithStreamOutput, with D3D11_SO_NO_RASTERIZED_STREAM
 * unless the case says otherwise, and check what each draw wrote:
 *
 *   bounds           a target of 256 bytes bound at offset 64 (room for 12
 *                    vertices of 16 bytes), 64 vertices drawn: the 12 that fit
 *                    are written, nothing before the offset, nothing into 8
 *                    buffers created after the target
 *   append           the append offset 0xffffffff on a target that was never
 *                    streamed into: the 4 vertices at offset 0, its filled size
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
 *                    them into A after the first draw's, and nothing into B
 *   append-draws     draws into one target with nothing between them, then
 *                    after binding it again at 0xffffffff, also after unbinding
 *                    it: each continues where the one before ended
 *   rewrite          a target bound at offset 0 again after a draw, with no
 *                    readback between: the second draw's vertices replace the
 *                    first draw's, 16 times
 *   overflow         triangles into a target with room for 5: the sixth is not
 *                    written, not even in part; SO_STATISTICS and
 *                    SO_OVERFLOW_PREDICATE count it
 *   topology         indexed draws write in index order, with 16-bit and 32-bit
 *                    indices, an index buffer offset, a start index and a base
 *                    vertex; strips are written as lists; a line list drops the
 *                    vertex that completes no line; instances follow each
 *                    other, also from a start instance other than 0
 *   slots            a declaration that writes two slots with a stride each:
 *                    both stop at the primitive the smaller one has no room
 *                    for, and nothing is written while one has no target
 *   draw-auto        DrawAuto draws what stream output wrote, from the offset
 *                    of the vertex buffer on
 *   indirect         indirect draws append and overflow like the others, the
 *                    queries count them, DrawAuto draws what they wrote, and a
 *                    draw after it appends
 *   deferred         a command list with a target, two draws and a query runs
 *                    twice: the second run continues after the first, and the
 *                    query has the counts of each run
 *   adjacency        the four topologies with adjacency write their primitives
 *                    without the adjacent vertices
 *   start-component  a draw with {POSITION .xyzw}, then a new geometry shader
 *                    with {POSITION .zw} drawn into another target: that
 *                    target holds .z and .w of each vertex and nothing else
 *   signature-blob   a shader created from an output signature blob streams
 *                    out; without a declaration such a blob is refused, and so
 *                    is one whose header gives a size past the length passed
 *   rasterized       with rasterized stream 0, a strip of two triangles over
 *                    the render target is streamed out as a list and drawn,
 *                    also from an output signature blob; it is drawn also
 *                    into a full target, by a draw and an indirect one; with
 *                    rasterized stream 1 it is streamed out and not drawn
 *   geometry-cut     an indexed triangle strip with a cut index at an even
 *                    position: every pipeline draws two quads with a gap
 *                    between them, through a geometry shader that passes its
 *                    triangles and the render target array index through
 *                    (into slice 1 of two) and one that does not pass its
 *                    triangles through, each with and without stream
 *                    output, without a geometry shader and through a vertex
 *                    shader that streams out; the stream output writes the
 *                    4 triangles of the quads and SO_STATISTICS counts 4
 *   geometry-cut-blocks  an indexed strip of 2600 indices with cuts, the
 *                    first and the last index among them, over the blocks of
 *                    1024 indices of DXMT's strip cut scan, of 16 and 32 bits,
 *                    with a start index and two instances, directly and
 *                    indirectly, through a geometry shader and through a
 *                    vertex shader, which also draws it as a line strip: the
 *                    primitives without a cut are written in order, each
 *                    triangle with the winding of its place in its strip
 *   geometry-pass-through  a D3D11 geometry shader that passes its triangles
 *                    and the render target array index through, with
 *                    rasterized stream 0, streams out and draws into the
 *                    slice it picks, and DrawAuto draws what it wrote
 *   geometry-rasterized  a geometry shader with rasterized stream 0 streams
 *                    out and draws, also into a full target; without a
 *                    rasterized stream it does not draw
 *   geometry-streams-rasterized  a geometry shader with a point in each of
 *                    four streams, each at its own place and with its own
 *                    colour in a register no other stream writes, created
 *                    with rasterized stream 0 to 3 in turn: only the
 *                    rasterized stream's point is drawn, in its colour, and
 *                    every stream appends its point to its slot
 *   geometry-uav     a geometry shader that streams out and adds 1 to a UAV
 *                    for each of 40 input points: the UAV holds 40, not
 *                    one count per pass of DXMT's geometry pipeline, and the
 *                    points are written in order
 *   geometry-vs-uav  a vertex shader that adds 1 to a UAV for each of 40
 *                    points, through a geometry shader that streams out: the
 *                    UAV holds 40, not one count per pass of DXMT's geometry
 *                    pipeline, and what the geometry shader emits is written
 *                    in order
 *   geometry-shapes  a triangle strip of 4 vertices without a cut, a line
 *                    strip, and a geometry shader of 4 instances are written
 *                    as lists, the instances in order
 *   geometry-strips  a geometry shader that cuts its triangle strips after
 *                    each triangle: 4 triangles for each input triangle
 *   geometry         a geometry shader that emits as many points as the
 *                    primitive's id & 3: the points of a draw, of two
 *                    instances and of a draw into the full target are
 *                    written in the order of the draws, and SO_STATISTICS
 *                    counts them
 *   geometry-order   two instances of 40 points through that shader, more
 *                    than DXMT's object threadgroup takes: in draw order
 *   geometry-large   64000 points through that shader: all 96000 points it
 *                    emits in draw order, and SO_STATISTICS counts them
 *   geometry-indirect  indirect draws through that shader append; one with
 *                    more input points than DXMT has room for writes nothing
 *   geometry-particles  a particle system: DrawAuto through a geometry shader
 *                    from the target the frame before filled, two targets
 *                    in turn for 5 frames, against the particles computed
 *                    here, and DrawAuto through a vertex shader of the last
 *   geometry-streams four point streams into four slots, each with its own
 *                    count of points and room, and a query of each stream
 *   geometry-streams-blocks  the four streams over 1024, 1025 and 2048
 *                    points, one and two blocks of DXMT's counts: every
 *                    stream's points and queries
 *
 * Every case runs on a device of its own, of feature level 11_0, or 11_1 for
 * geometry-uav and geometry-vs-uav, which gives geometry and vertex shaders
 * UAVs. Every buffer but the targets of geometry-large and
 * geometry-streams-blocks starts filled with a pattern.
 * The cases up to geometry-strips read their targets back between draws,
 * which waits for the GPU; the ones after it draw without a readback between,
 * so DXMT has to order what two draws of a render pass write.
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
 * ATTRIB4, ATTRIB3 and ATTRIB4 packed into one register), the geometry
 * shader of test_stream_output, the vs_code of test_swapchain_flip, the
 * ps_color_code of test_draw_depth_only, the vs_code and gs_code of
 * test_so_statistics_query and the gs_4_code and gs_5_code of
 * test_layered_rendering in Wine's dlls/d3d11/tests/d3d11.c, the gs_code of
 * test_stream_output_resume in its dlls/d3d10core/tests/d3d10core.c, and
 * geometry and pixel shaders assembled here. */

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

/* float4 main(float4 position : POSITION) : SV_POSITION { return position; } */
static const DWORD position_vs_code[] = {
    0x43425844, 0xa7a2f22d, 0x83ff2560, 0xe61638bd, 0x87e3ce90, 0x00000001, 0x000000d8, 0x00000003,
    0x0000002c, 0x00000060, 0x00000094, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000f0f, 0x49534f50, 0x4e4f4954, 0xababab00,
    0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000001, 0x00000003,
    0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49, 0x52444853, 0x0000003c, 0x00010040,
    0x0000000f, 0x0300005f, 0x001010f2, 0x00000000, 0x04000067, 0x001020f2, 0x00000000, 0x00000001,
    0x05000036, 0x001020f2, 0x00000000, 0x00101e46, 0x00000000, 0x0100003e,
};

/* float4 main(float4 position : SV_POSITION) : SV_Target
 * { return float4(0.0, 1.0, 0.0, 1.0); } */
static const DWORD green_ps_code[] = {
    0x43425844, 0x30240e72, 0x012f250c, 0x8673c6ea, 0x392e4cec, 0x00000001, 0x000000d4, 0x00000003,
    0x0000002c, 0x00000060, 0x00000094, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49,
    0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003,
    0x00000000, 0x0000000f, 0x545f5653, 0x65677261, 0xabab0074, 0x52444853, 0x00000038, 0x00000040,
    0x0000000e, 0x03000065, 0x001020f2, 0x00000000, 0x08000036, 0x001020f2, 0x00000000, 0x00004002,
    0x00000000, 0x3f800000, 0x00000000, 0x3f800000, 0x0100003e,
};

/* float4 main(uint id : SV_VertexID) : custom { return (float4)id; } */
static const DWORD id_vs_code[] = {
    0x43425844, 0x8b0e47b9, 0x6efc9512, 0xd55ca6ff, 0x487c5ef2, 0x00000001, 0x000000d4, 0x00000003,
    0x0000002c, 0x00000060, 0x00000090, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000006, 0x00000001, 0x00000000, 0x00000101, 0x565f5653, 0x65747265, 0x00444978,
    0x4e47534f, 0x00000028, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003,
    0x00000000, 0x0000000f, 0x74737563, 0xab006d6f, 0x52444853, 0x0000003c, 0x00010040, 0x0000000f,
    0x04000060, 0x00101012, 0x00000000, 0x00000006, 0x03000065, 0x001020f2, 0x00000000, 0x05000056,
    0x001020f2, 0x00000000, 0x00101006, 0x00000000, 0x0100003e,
};

/* The chunks of a geometry shader whose number of vertices depends on the
 * primitive, assembled here as gs_4_0; build_container makes the bytecode:
 * struct vertex { float4 data : custom; };
 * [maxvertexcount(4)]
 * void main(point vertex input[1], uint id : SV_PrimitiveID, inout PointStream<vertex> output)
 * { for (uint i = 0; i < (id & 3); i++) { vertex o = input[0]; o.data.y = i; output.Append(o); } } */
static const uint32_t count_gs_isgn[] = {
    0x4e475349, 0x00000050, 0x00000002, 0x00000008, 0x00000038, 0x00000000, 0x00000000, 0x00000003,
    0x00000000, 0x00000f0f, 0x0000003f, 0x00000000, 0x00000007, 0x00000001, 0xffffffff, 0x00000101,
    0x74737563, 0x53006d6f, 0x72505f56, 0x74696d69, 0x49657669, 0xabab0044,
};
static const uint32_t count_gs_osgn[] = {
    0x4e47534f, 0x00000028, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003,
    0x00000000, 0x0000000f, 0x74737563, 0xab006d6f,
};
static const uint32_t count_gs_shdr[] = {
    0x52444853, 0x000000f0, 0x00020040, 0x0000003c,
    0x0200005f, 0x0000b000,                                     /* dcl_input vPrim */
    0x0400005f, 0x002010f2, 0x00000001, 0x00000000,             /* dcl_input v[1][0].xyzw */
    0x02000068, 0x00000001,                                     /* dcl_temps 1 */
    0x0100085d,                                                 /* dcl_inputprimitive point */
    0x0100085c,                                                 /* dcl_outputtopology pointlist */
    0x03000065, 0x001020f2, 0x00000000,                         /* dcl_output o0.xyzw */
    0x0200005e, 0x00000004,                                     /* dcl_maxout 4 */
    0x05000036, 0x00100012, 0x00000000, 0x00004001, 0x00000000, /* mov r0.x, l(0) */
    0x06000001, 0x00100042, 0x00000000, 0x0000b001, 0x00004001, 0x00000003, /* and r0.z, vPrim, l(3) */
    0x01000030,                                                 /* loop */
    0x07000050, 0x00100022, 0x00000000, 0x0010000a, 0x00000000, 0x0010002a, 0x00000000, /* uge r0.y, r0.x, r0.z */
    0x03040003, 0x0010001a, 0x00000000,                         /* breakc_nz r0.y */
    0x06000036, 0x001020d2, 0x00000000, 0x00201e06, 0x00000000, 0x00000000, /* mov o0.xzw, v[0][0].xxzw */
    0x05000056, 0x00102022, 0x00000000, 0x0010000a, 0x00000000, /* utof o0.y, r0.x */
    0x01000013,                                                 /* emit */
    0x0700001e, 0x00100012, 0x00000000, 0x0010000a, 0x00000000, 0x00004001, 0x00000001, /* iadd r0.x, r0.x, l(1) */
    0x01000016,                                                 /* endloop */
    0x0100003e,                                                 /* ret */
};

/* The ISGN and OSGN chunks of triangle_gs_code, SV_POSITION in and out, which
 * the shader chunks below are assembled with */
#define TRIANGLE_GS_ISGN ((const uint32_t *)triangle_gs_code + 11)
#define TRIANGLE_GS_OSGN ((const uint32_t *)triangle_gs_code + 24)
#define TRIANGLE_GS_SIGNATURE_WORDS 13

/* SV_POSITION in register 0 and a uint LAYER .x in register 1: the output
 * signature of layer_vs and the input signature of layer_pass_gs */
#define LAYER_SIGNATURE(tag, position_mask, layer_mask)                                                      \
    tag, 0x0000004c, 0x00000002, 0x00000008,                                                               \
    0x00000038, 0x00000000, 0x00000001, 0x00000003, 0x00000000, position_mask,                            \
    0x00000044, 0x00000000, 0x00000000, 0x00000001, 0x00000001, layer_mask,                               \
    0x505f5653, 0x5449534f, 0x004e4f49, 0x4559414c, 0xabab0052 /* "SV_POSITION", "LAYER" */
static const uint32_t layer_vs_osgn[] = { LAYER_SIGNATURE(0x4e47534f, 0x0000000f, 0x00000e01) };
static const uint32_t layer_gs_isgn[] = { LAYER_SIGNATURE(0x4e475349, 0x00000f0f, 0x00000101) };

/* SV_POSITION in register 0 and SV_RenderTargetArrayIndex .x in register 1 */
static const uint32_t layer_gs_osgn[] = {
    0x4e47534f, 0x00000060, 0x00000002, 0x00000008,
    0x00000038, 0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x0000000f,
    0x00000044, 0x00000000, 0x00000004, 0x00000001, 0x00000001, 0x00000e01,
    0x505f5653, 0x5449534f, 0x004e4f49,                         /* "SV_POSITION" */
    0x525f5653, 0x65646e65, 0x72615472, 0x41746567, 0x79617272, 0x65646e49, 0xabab0078, /* "SV_RenderTargetArrayIndex" */
};

/* float4 position : POSITION in, the position and LAYER 1 out */
static const uint32_t layer_vs_shdr[] = {
    0x52444853, 0x0000005c, 0x00010040, 0x00000017,
    0x0300005f, 0x001010f2, 0x00000000,                         /* dcl_input v0.xyzw */
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */
    0x03000065, 0x00102012, 0x00000001,                         /* dcl_output o1.x */
    0x05000036, 0x001020f2, 0x00000000, 0x00101e46, 0x00000000, /* mov o0, v0 */
    0x05000036, 0x00102012, 0x00000001, 0x00004001, 0x00000001, /* mov o1.x, l(1) */
    0x0100003e,                                                 /* ret */
};

/* A geometry shader that passes its triangle through, the three vertices
 * emitted one after another, with LAYER as SV_RenderTargetArrayIndex. DXMT's
 * CheckGSBBIsPassThrough accepts no shader today (#199), so DXMT draws it
 * through its geometry pipeline; the render target array index makes it pass
 * DXMT's signature check for a pass-through once that is fixed, which only a
 * shader that passes the render target array or viewport index through
 * does. */
#define LAYER_PASS_VERTEX(v)                                                                                 \
    0x06000036, 0x001020f2, 0x00000000, 0x00201e46, v, 0x00000000, /* mov o0, v[v][0] */                   \
    0x06000036, 0x00102012, 0x00000001, 0x0020100a, v, 0x00000001, /* mov o1.x, v[v][1].x */               \
    0x01000013                                                     /* emit */
static const uint32_t layer_pass_gs_shdr[] = {
    0x52444853, 0x000000fc, 0x00020040, 0x0000003f,
    0x05000061, 0x002010f2, 0x00000003, 0x00000000, 0x00000001, /* dcl_input_siv v[3][0].xyzw, position */
    0x0400005f, 0x00201012, 0x00000003, 0x00000001,             /* dcl_input v[3][1].x */
    0x0100185d,                                                 /* dcl_inputprimitive triangle */
    0x0100285c,                                                 /* dcl_outputtopology trianglestrip */
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */
    0x04000067, 0x00102012, 0x00000001, 0x00000004,             /* dcl_output_siv o1.x, rendertarget_array_index */
    0x0200005e, 0x00000003,                                     /* dcl_maxout 3 */
    LAYER_PASS_VERTEX(0), LAYER_PASS_VERTEX(1), LAYER_PASS_VERTEX(2),
    0x0100003e,                                                 /* ret */
};

/* The shader of triangle_gs_code with output topology <topology> and, unless
 * <loop_only>, the first vertex emitted once more after the loop: a strip of
 * 4 vertices that no cut ends, or a line strip of 3 */
#define LOOP_GS_SHDR(size, length, topology, maxout)                                                         \
    0x52444853, size, 0x00020040, length,                                                                  \
    0x05000061, 0x002010f2, 0x00000003, 0x00000000, 0x00000001, /* dcl_input_siv v[3][0].xyzw, position */ \
    0x02000068, 0x00000001,                                     /* dcl_temps 1 */                         \
    0x0100185d,                                                 /* dcl_inputprimitive triangle */         \
    topology,                                                   /* dcl_outputtopology */                  \
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */    \
    0x0200005e, maxout,                                         /* dcl_maxout */                          \
    0x05000036, 0x00100012, 0x00000000, 0x00004001, 0x00000000, /* mov r0.x, l(0) */                      \
    0x01000030,                                                 /* loop */                                \
    0x07000050, 0x00100022, 0x00000000, 0x0010000a, 0x00000000, 0x00004001, 0x00000003, /* uge r0.y, r0.x, l(3) */ \
    0x03040003, 0x0010001a, 0x00000000,                         /* breakc_nz r0.y */                      \
    0x07000036, 0x001020f2, 0x00000000, 0x00a01e46, 0x0010000a, 0x00000000, 0x00000000, /* mov o0, v[r0.x][0] */ \
    0x01000013,                                                 /* emit */                                \
    0x0700001e, 0x00100012, 0x00000000, 0x0010000a, 0x00000000, 0x00004001, 0x00000001, /* iadd r0.x, r0.x, l(1) */ \
    0x01000016                                                  /* endloop */
static const uint32_t strip4_gs_shdr[] = {
    LOOP_GS_SHDR(0x000000e4, 0x00000039, 0x0100285c, 0x00000004), /* trianglestrip, maxout 4 */
    0x06000036, 0x001020f2, 0x00000000, 0x00201e46, 0x00000000, 0x00000000, /* mov o0, v[0][0] */
    0x01000013,                                                 /* emit */
    0x0100003e,                                                 /* ret */
};
static const uint32_t line_gs_shdr[] = {
    LOOP_GS_SHDR(0x000000c8, 0x00000032, 0x0100185c, 0x00000003), /* linestrip, maxout 3 */
    0x0100003e,                                                 /* ret */
};

/* A particle system: each point ages by 1, and a point younger than 2 adds
 * a point of its age, with y one higher
 * [maxvertexcount(2)]
 * void main(point float4 input[1] : SV_POSITION, inout PointStream<float4 : SV_POSITION> output)
 * { output.Append(float4(input[0].x + 1, input[0].yzw));
 *   if (input[0].x < 2) output.Append(float4(input[0].x + 1, input[0].y + 1, input[0].zw)); } */
static const uint32_t particle_gs_shdr[] = {
    0x52444853, 0x00000118, 0x00020040, 0x00000046,
    0x05000061, 0x002010f2, 0x00000001, 0x00000000, 0x00000001, /* dcl_input_siv v[1][0].xyzw, position */
    0x02000068, 0x00000001,                                     /* dcl_temps 1 */
    0x0100085d,                                                 /* dcl_inputprimitive point */
    0x0100085c,                                                 /* dcl_outputtopology pointlist */
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */
    0x0200005e, 0x00000002,                                     /* dcl_maxout 2 */
    0x08000000, 0x00100012, 0x00000000, 0x0020100a, 0x00000000, 0x00000000, 0x00004001, 0x3f800000, /* add r0.x, v[0][0].x, l(1.0) */
    0x05000036, 0x00102012, 0x00000000, 0x0010000a, 0x00000000, /* mov o0.x, r0.x */
    0x06000036, 0x001020e2, 0x00000000, 0x00201e56, 0x00000000, 0x00000000, /* mov o0.yzw, v[0][0].yyzw */
    0x01000013,                                                 /* emit */
    0x08000031, 0x00100022, 0x00000000, 0x0020100a, 0x00000000, 0x00000000, 0x00004001, 0x40000000, /* lt r0.y, v[0][0].x, l(2.0) */
    0x0304001f, 0x0010001a, 0x00000000,                         /* if_nz r0.y */
    0x08000000, 0x00102022, 0x00000000, 0x0020101a, 0x00000000, 0x00000000, 0x00004001, 0x3f800000, /* add o0.y, v[0][0].y, l(1.0) */
    0x05000036, 0x00102012, 0x00000000, 0x0010000a, 0x00000000, /* mov o0.x, r0.x */
    0x06000036, 0x001020c2, 0x00000000, 0x00201ea6, 0x00000000, 0x00000000, /* mov o0.zw, v[0][0].zzzw */
    0x01000013,                                                 /* emit */
    0x01000015,                                                 /* endif */
    0x0100003e,                                                 /* ret */
};

/* The gs_5_code of test_layered_rendering: the triangle with
 * SV_RenderTargetArrayIndex from the constant buffer plus SV_GSInstanceID,
 * for each of 4 instances
 * uint layer_offset;
 * [instance(4)]
 * [maxvertexcount(3)]
 * void main(triangle gs_in vin[3], in uint instance_id : SV_GSInstanceID, inout TriangleStream<gs_out> vout)
 * { gs_out o; o.layer = layer_offset + instance_id;
 *   for (uint i = 0; i < 3; ++i) { o.pos = vin[i].pos; vout.Append(o); } } */
static const DWORD instances_gs_code[] = {
    0x43425844, 0xb52da162, 0x9a13d8ee, 0xf7c30b50, 0xe80bc2e7, 0x00000001, 0x00000218, 0x00000003,
    0x0000002c, 0x00000060, 0x000000d0, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x00000f0f, 0x505f5653, 0x7469736f, 0x006e6f69,
    0x3547534f, 0x00000068, 0x00000002, 0x00000008, 0x00000000, 0x00000040, 0x00000000, 0x00000001,
    0x00000003, 0x00000000, 0x0000000f, 0x00000000, 0x0000004c, 0x00000000, 0x00000004, 0x00000001,
    0x00000001, 0x00000e01, 0x505f5653, 0x7469736f, 0x006e6f69, 0x525f5653, 0x65646e65, 0x72615472,
    0x41746567, 0x79617272, 0x65646e49, 0xabab0078, 0x58454853, 0x00000140, 0x00020050, 0x00000050,
    0x0100086a, 0x04000059, 0x00208e46, 0x00000000, 0x00000001, 0x05000061, 0x002010f2, 0x00000003,
    0x00000000, 0x00000001, 0x0200005f, 0x00025000, 0x02000068, 0x00000001, 0x020000ce, 0x00000004,
    0x0100185d, 0x0300008f, 0x00110000, 0x00000000, 0x0100285c, 0x04000067, 0x001020f2, 0x00000000,
    0x00000001, 0x04000067, 0x00102012, 0x00000001, 0x00000004, 0x0200005e, 0x00000003, 0x0700001e,
    0x00100012, 0x00000000, 0x0020800a, 0x00000000, 0x00000000, 0x0002500a, 0x05000036, 0x00100022,
    0x00000000, 0x00004001, 0x00000000, 0x01000030, 0x07000050, 0x00100042, 0x00000000, 0x0010001a,
    0x00000000, 0x00004001, 0x00000003, 0x03040003, 0x0010002a, 0x00000000, 0x07000036, 0x001020f2,
    0x00000000, 0x00a01e46, 0x0010001a, 0x00000000, 0x00000000, 0x05000036, 0x00102012, 0x00000001,
    0x0010000a, 0x00000000, 0x03000075, 0x00110000, 0x00000000, 0x0700001e, 0x00100022, 0x00000000,
    0x0010001a, 0x00000000, 0x00004001, 0x00000001, 0x01000016, 0x0100003e,
};

/* The gs_4_code of test_layered_rendering: 4 triangle strips of 3 vertices,
 * SV_Position and a SV_RenderTargetArrayIndex, for each input triangle */
static const DWORD strips_gs_code[] = {
    0x43425844, 0x7eabd7c5, 0x8af1468e, 0xd585cade, 0xfe0d761d, 0x00000001, 0x00000250, 0x00000003,
    0x0000002c, 0x00000060, 0x000000c8, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x00000f0f, 0x505f5653, 0x7469736f, 0x006e6f69,
    0x4e47534f, 0x00000060, 0x00000002, 0x00000008, 0x00000038, 0x00000000, 0x00000001, 0x00000003,
    0x00000000, 0x0000000f, 0x00000044, 0x00000000, 0x00000004, 0x00000001, 0x00000001, 0x00000e01,
    0x505f5653, 0x7469736f, 0x006e6f69, 0x525f5653, 0x65646e65, 0x72615472, 0x41746567, 0x79617272,
    0x65646e49, 0xabab0078, 0x52444853, 0x00000180, 0x00020040, 0x00000060, 0x04000059, 0x00208e46,
    0x00000000, 0x00000001, 0x05000061, 0x002010f2, 0x00000003, 0x00000000, 0x00000001, 0x02000068,
    0x00000001, 0x0100185d, 0x0100285c, 0x04000067, 0x001020f2, 0x00000000, 0x00000001, 0x04000067,
    0x00102012, 0x00000001, 0x00000004, 0x0200005e, 0x0000000c, 0x05000036, 0x00100012, 0x00000000,
    0x00004001, 0x00000000, 0x01000030, 0x07000050, 0x00100022, 0x00000000, 0x0010000a, 0x00000000,
    0x00004001, 0x00000004, 0x03040003, 0x0010001a, 0x00000000, 0x0800001e, 0x00100022, 0x00000000,
    0x0020800a, 0x00000000, 0x00000000, 0x0010000a, 0x00000000, 0x05000036, 0x00100042, 0x00000000,
    0x00004001, 0x00000000, 0x01000030, 0x07000050, 0x00100082, 0x00000000, 0x0010002a, 0x00000000,
    0x00004001, 0x00000003, 0x03040003, 0x0010003a, 0x00000000, 0x07000036, 0x001020f2, 0x00000000,
    0x00a01e46, 0x0010002a, 0x00000000, 0x00000000, 0x05000036, 0x00102012, 0x00000001, 0x0010001a,
    0x00000000, 0x01000013, 0x0700001e, 0x00100042, 0x00000000, 0x0010002a, 0x00000000, 0x00004001,
    0x00000001, 0x01000016, 0x01000009, 0x0700001e, 0x00100012, 0x00000000, 0x0010000a, 0x00000000,
    0x00004001, 0x00000001, 0x01000016, 0x0100003e,
};

/* The gs_code of test_stream_output_resume in Wine's d3d10core.c: the
 * triangle with so_output from its constant buffer
 * float4 constant;
 * struct vertex { float4 position : SV_POSITION; };
 * struct element { float4 position : SV_POSITION; float4 so_output : so_output; };
 * [maxvertexcount(3)]
 * void main(triangle vertex input[3], inout TriangleStream<element> output)
 * { element o; o.so_output = constant;
 *   o.position = input[0].position; output.Append(o);
 *   o.position = input[1].position; output.Append(o);
 *   o.position = input[2].position; output.Append(o); } */
static const DWORD constant_gs_code[] = {
    0x43425844, 0x76f5793f, 0x08760f12, 0xb730b512, 0x3728e75c, 0x00000001, 0x000001b8, 0x00000003,
    0x0000002c, 0x00000060, 0x000000b8, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x00000f0f, 0x505f5653, 0x5449534f, 0x004e4f49,
    0x4e47534f, 0x00000050, 0x00000002, 0x00000008, 0x00000038, 0x00000000, 0x00000001, 0x00000003,
    0x00000000, 0x0000000f, 0x00000044, 0x00000000, 0x00000000, 0x00000003, 0x00000001, 0x0000000f,
    0x505f5653, 0x5449534f, 0x004e4f49, 0x6f5f6f73, 0x75707475, 0xabab0074, 0x52444853, 0x000000f8,
    0x00020040, 0x0000003e, 0x04000059, 0x00208e46, 0x00000000, 0x00000001, 0x05000061, 0x002010f2,
    0x00000003, 0x00000000, 0x00000001, 0x0100185d, 0x0100285c, 0x04000067, 0x001020f2, 0x00000000,
    0x00000001, 0x03000065, 0x001020f2, 0x00000001, 0x0200005e, 0x00000003, 0x06000036, 0x001020f2,
    0x00000000, 0x00201e46, 0x00000000, 0x00000000, 0x06000036, 0x001020f2, 0x00000001, 0x00208e46,
    0x00000000, 0x00000000, 0x01000013, 0x06000036, 0x001020f2, 0x00000000, 0x00201e46, 0x00000001,
    0x00000000, 0x06000036, 0x001020f2, 0x00000001, 0x00208e46, 0x00000000, 0x00000000, 0x01000013,
    0x06000036, 0x001020f2, 0x00000000, 0x00201e46, 0x00000002, 0x00000000, 0x06000036, 0x001020f2,
    0x00000001, 0x00208e46, 0x00000000, 0x00000000, 0x01000013, 0x0100003e,
};

/* The gs_code of test_so_statistics_query: the point into each of 4 streams
 * while the primitive's id is below the stream's count
 * struct vertex { float4 data : custom; };
 * uint4 vertex_count;
 * [maxvertexcount(32)]
 * void main(point vertex input[1], uint id : SV_PrimitiveID,
 *         inout PointStream<vertex> output0, inout PointStream<vertex> output1,
 *         inout PointStream<vertex> output2, inout PointStream<vertex> output3)
 * { if (id < vertex_count.x) output0.Append(input[0]);
 *   if (id < vertex_count.y) output1.Append(input[0]);
 *   if (id < vertex_count.z) output2.Append(input[0]);
 *   if (id < vertex_count.w) output3.Append(input[0]); } */
static const DWORD streams_gs_code[] = {
    0x43425844, 0xd616829d, 0x4355ce2a, 0xd71909e5, 0xdc916d4c, 0x00000001, 0x000002bc, 0x00000003,
    0x0000002c, 0x00000084, 0x0000010c, 0x4e475349, 0x00000050, 0x00000002, 0x00000008, 0x00000038,
    0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000f0f, 0x0000003f, 0x00000000, 0x00000007,
    0x00000001, 0xffffffff, 0x00000101, 0x74737563, 0x53006d6f, 0x72505f56, 0x74696d69, 0x49657669,
    0xabab0044, 0x3547534f, 0x00000080, 0x00000004, 0x00000008, 0x00000000, 0x00000078, 0x00000000,
    0x00000000, 0x00000003, 0x00000000, 0x0000000f, 0x00000001, 0x00000078, 0x00000000, 0x00000000,
    0x00000003, 0x00000000, 0x0000000f, 0x00000002, 0x00000078, 0x00000000, 0x00000000, 0x00000003,
    0x00000000, 0x0000000f, 0x00000003, 0x00000078, 0x00000000, 0x00000000, 0x00000003, 0x00000000,
    0x0000000f, 0x74737563, 0xab006d6f, 0x58454853, 0x000001a8, 0x00020050, 0x0000006a, 0x0100086a,
    0x04000059, 0x00208e46, 0x00000000, 0x00000001, 0x0400005f, 0x002010f2, 0x00000001, 0x00000000,
    0x0200005f, 0x0000b000, 0x02000068, 0x00000001, 0x0100085d, 0x0300008f, 0x00110000, 0x00000000,
    0x0100085c, 0x03000065, 0x001020f2, 0x00000000, 0x0300008f, 0x00110000, 0x00000001, 0x0100085c,
    0x03000065, 0x001020f2, 0x00000000, 0x0300008f, 0x00110000, 0x00000002, 0x0100085c, 0x03000065,
    0x001020f2, 0x00000000, 0x0300008f, 0x00110000, 0x00000003, 0x0100085c, 0x03000065, 0x001020f2,
    0x00000000, 0x0200005e, 0x00000020, 0x0700004f, 0x001000f2, 0x00000000, 0x0000b001, 0x00208e46,
    0x00000000, 0x00000000, 0x0304001f, 0x0010000a, 0x00000000, 0x06000036, 0x001020f2, 0x00000000,
    0x00201e46, 0x00000000, 0x00000000, 0x03000075, 0x00110000, 0x00000000, 0x01000015, 0x0304001f,
    0x0010001a, 0x00000000, 0x06000036, 0x001020f2, 0x00000000, 0x00201e46, 0x00000000, 0x00000000,
    0x03000075, 0x00110000, 0x00000001, 0x01000015, 0x0304001f, 0x0010002a, 0x00000000, 0x06000036,
    0x001020f2, 0x00000000, 0x00201e46, 0x00000000, 0x00000000, 0x03000075, 0x00110000, 0x00000002,
    0x01000015, 0x0304001f, 0x0010003a, 0x00000000, 0x06000036, 0x001020f2, 0x00000000, 0x00201e46,
    0x00000000, 0x00000000, 0x03000075, 0x00110000, 0x00000003, 0x01000015, 0x0100003e,
};

/* The shader chunk of a geometry shader that adds 1 to a UAV for each input
 * point and emits the point, assembled here as gs_5_0 with the ISGN and OSGN
 * chunks of count_gs:
 * RWByteAddressBuffer counter : register(u1);
 * [maxvertexcount(1)]
 * void main(point vertex input[1], inout PointStream<vertex> output)
 * { counter.InterlockedAdd(0, 1); output.Append(input[0]); } */
static const uint32_t uav_gs_shdr[] = {
    0x58454853, 0x0000007c, 0x00020050, 0x0000001f,
    0x0300009d, 0x0011e000, 0x00000001,                         /* dcl_uav_raw u1 */
    0x0400005f, 0x002010f2, 0x00000001, 0x00000000,             /* dcl_input v[1][0].xyzw */
    0x0100085d,                                                 /* dcl_inputprimitive point */
    0x0100085c,                                                 /* dcl_outputtopology pointlist */
    0x03000065, 0x001020f2, 0x00000000,                         /* dcl_output o0.xyzw */
    0x0200005e, 0x00000001,                                     /* dcl_maxout 1 */
    0x070000ad, 0x0011e000, 0x00000001, 0x00004001, 0x00000000, 0x00004001, 0x00000001, /* atomic_iadd u1, l(0), l(1) */
    0x06000036, 0x001020f2, 0x00000000, 0x00201e46, 0x00000000, 0x00000000, /* mov o0, v[0][0] */
    0x01000013,                                                 /* emit */
    0x0100003e,                                                 /* ret */
};

/* The shader chunk of a vertex shader that adds 1 to a UAV for each vertex,
 * assembled here as vs_5_0 with the ISGN and OSGN chunks of id_vs_code:
 * RWByteAddressBuffer counter : register(u1);
 * float4 main(uint id : SV_VertexID) : custom
 * { counter.InterlockedAdd(0, 1); return (float4)id; } */
static const uint32_t uav_vs_shdr[] = {
    0x58454853, 0x00000064, 0x00010050, 0x00000019,
    0x0300009d, 0x0011e000, 0x00000001,                         /* dcl_uav_raw u1 */
    0x04000060, 0x00101012, 0x00000000, 0x00000006,             /* dcl_input_sgv v0.x, vertex_id */
    0x03000065, 0x001020f2, 0x00000000,                         /* dcl_output o0.xyzw */
    0x070000ad, 0x0011e000, 0x00000001, 0x00004001, 0x00000000, 0x00004001, 0x00000001, /* atomic_iadd u1, l(0), l(1) */
    0x05000056, 0x001020f2, 0x00000000, 0x00101006, 0x00000000, /* utof o0.xyzw, v0.xxxx */
    0x0100003e,                                                 /* ret */
};

/* The OSG5 and shader chunks of a geometry shader that emits one point into
 * each of 4 streams, assembled here as gs_5_0 with the ISGN chunk of
 * count_gs. Stream s writes SV_Position into o0, the centre of pixel
 * (8 + 16 * s, 32) of the 64x64 render target, and COLOR into o<1 + s>, a
 * register no other stream writes: green, blue, white and yellow. */
static const uint32_t raster_streams_osg5[] = {
    0x3547534f, 0x000000fc, 0x00000008, 0x00000008,
#define RASTER_STREAM_OSG5(s) \
    s, 0x000000e8, 0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x0000000f, /* SV_Position o0 */ \
    s, 0x000000f4, 0x00000000, 0x00000000, 0x00000003, 1 + s, 0x0000000f       /* COLOR o<1 + s> */
    RASTER_STREAM_OSG5(0), RASTER_STREAM_OSG5(1), RASTER_STREAM_OSG5(2), RASTER_STREAM_OSG5(3),
#undef RASTER_STREAM_OSG5
    0x505f5653, 0x7469736f, 0x006e6f69, 0x4f4c4f43, 0xabab0052, /* "SV_Position", "COLOR" */
};
static const uint32_t raster_streams_shdr[] = {
    0x58454853, 0x00000208, 0x00020050, 0x00000082,
    0x0400005f, 0x002010f2, 0x00000001, 0x00000000,             /* dcl_input v[1][0].xyzw */
    0x0100085d,                                                 /* dcl_inputprimitive point */
#define RASTER_STREAM_DCL(s) \
    0x0300008f, 0x00110000, s,                                  /* dcl_stream m<s> */ \
    0x0100085c,                                                 /* dcl_outputtopology pointlist */ \
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */ \
    0x03000065, 0x001020f2, 1 + s                               /* dcl_output o<1 + s>.xyzw */
    RASTER_STREAM_DCL(0), RASTER_STREAM_DCL(1), RASTER_STREAM_DCL(2), RASTER_STREAM_DCL(3),
#undef RASTER_STREAM_DCL
    0x0200005e, 0x00000004,                                     /* dcl_maxout 4 */
#define RASTER_STREAM_EMIT(s, x, r, g, b) \
    0x08000036, 0x001020f2, 0x00000000, 0x00004002, x, 0xbc800000, 0x3f000000, 0x3f800000, /* mov o0, l(x, y, 0.5, 1.0) */ \
    0x08000036, 0x001020f2, 1 + s, 0x00004002, r, g, b, 0x3f800000, /* mov o<1 + s>, l(r, g, b, 1.0) */ \
    0x03000075, 0x00110000, s                                   /* emit_stream m<s> */
    RASTER_STREAM_EMIT(0, 0xbf3c0000, 0x00000000, 0x3f800000, 0x00000000), /* x -0.734375, green */
    RASTER_STREAM_EMIT(1, 0xbe700000, 0x00000000, 0x00000000, 0x3f800000), /* x -0.234375, blue */
    RASTER_STREAM_EMIT(2, 0x3e880000, 0x3f800000, 0x3f800000, 0x3f800000), /* x 0.265625, white */
    RASTER_STREAM_EMIT(3, 0x3f440000, 0x3f800000, 0x3f800000, 0x00000000), /* x 0.765625, yellow */
#undef RASTER_STREAM_EMIT
    0x0100003e,                                                 /* ret */
};

#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))
#define CONTAINER_WORDS 256
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

#define LARGE_SIZE 2048
#define LARGE_WORDS (LARGE_SIZE / 4)

static ID3D11Buffer *create_buffer_bind(UINT size, uint32_t fill, UINT bind)
{
    static uint32_t data[LARGE_WORDS];
    D3D11_BUFFER_DESC desc = { size, D3D11_USAGE_DEFAULT, bind, 0, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { data, 0, 0 };
    ID3D11Buffer *buffer;
    unsigned i;

    if (size > LARGE_SIZE) fail(2, "a buffer of %u bytes", size);
    for (i = 0; i < size / 4; i++) data[i] = fill;
    check_hr(ID3D11Device_CreateBuffer(device, &desc, &init, &buffer), "CreateBuffer");
    return buffer;
}

static ID3D11Buffer *create_buffer(UINT size, uint32_t fill)
{
    return create_buffer_bind(size, fill, D3D11_BIND_STREAM_OUTPUT);
}

/* A buffer with <data>, for indices (<bind>) or the arguments of indirect
 * draws (<misc>) */
static ID3D11Buffer *create_data_buffer(const void *data, UINT size, UINT bind, UINT misc)
{
    D3D11_BUFFER_DESC desc = { size, D3D11_USAGE_DEFAULT, bind, 0, misc, 0 };
    D3D11_SUBRESOURCE_DATA init = { data, 0, 0 };
    ID3D11Buffer *buffer;

    check_hr(ID3D11Device_CreateBuffer(device, &desc, &init, &buffer), "CreateBuffer");
    return buffer;
}

static ID3D11Query *create_query(D3D11_QUERY type)
{
    D3D11_QUERY_DESC desc = { type, 0 };
    ID3D11Query *query;

    check_hr(ID3D11Device_CreateQuery(device, &desc, &query), "CreateQuery");
    return query;
}

/* Wait for the data of <query>, up to 5 seconds */
static void get_query_data(ID3D11Query *query, void *data, UINT size)
{
    HRESULT hr = S_FALSE;
    unsigned i;

    for (i = 0; i < 500 && hr == S_FALSE; i++)
    {
        hr = ID3D11DeviceContext_GetData(context, (ID3D11Asynchronous *)query, data, size, 0);
        if (hr == S_FALSE) Sleep(10);
    }
    if (hr != S_OK) fail(2, "GetData returned %#lx", hr);
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

/* Copy the <size> bytes of <buffer> into <out>. The Map waits for the GPU to
 * finish every draw before it. */
static void read_buffer_size(ID3D11Buffer *buffer, uint32_t *out, UINT size)
{
    D3D11_BUFFER_DESC desc = { size, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ, 0, 0 };
    D3D11_MAPPED_SUBRESOURCE map;
    ID3D11Buffer *staging;

    check_hr(ID3D11Device_CreateBuffer(device, &desc, NULL, &staging), "CreateBuffer");
    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging, (ID3D11Resource *)buffer);
    check_hr(ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &map), "Map");
    memcpy(out, map.pData, size);
    ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, 0);
    ID3D11Buffer_Release(staging);
}

static void read_buffer(ID3D11Buffer *buffer, uint32_t *out)
{
    read_buffer_size(buffer, out, TARGET_SIZE);
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

/* Whether the <n> slots of <words> words from slot <first> on hold the
 * vertices v[0] to v[n - 1] of set s */
static int has_list(const uint32_t *data, unsigned words, unsigned first, unsigned n, unsigned s, const unsigned *v)
{
    unsigned i;

    for (i = 0; i < n; i++)
        if (!is_vertex(data, (first + i) * words, s, v[i])) return 0;
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

/* <level> is the feature level of the device, 11_0 when 0 */
static void setup(D3D_FEATURE_LEVEL level)
{
    static const D3D11_INPUT_ELEMENT_DESC layout_desc[] = {
        { "POSITION", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 16, D3D11_INPUT_PER_VERTEX_DATA, 0 },
        { "COLOR", 1, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 32, D3D11_INPUT_PER_VERTEX_DATA, 0 },
    };
    static const D3D11_SO_DECLARATION_ENTRY so_decl[] = { { 0, "POSITION", 0, 0, 4, 0 } };
    D3D11_TEXTURE2D_DESC rt_desc = { 64, 64, 1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT,
                                     D3D11_BIND_RENDER_TARGET, 0, 0 };
    UINT stride = 16;

    if (!level) level = D3D_FEATURE_LEVEL_11_0;
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

/* The offset 0xffffffff appends at the filled size of the buffer, which is 0
 * for one that was never streamed into. */
static void append_case(void)
{
    static uint32_t data[TARGET_WORDS];
    unsigned changed;

    bind_pipeline(vb0);
    bind_target(target, 0xffffffff);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(target, data);
    changed = count_changed(data, 0, TARGET_WORDS, TARGET_FILL);
    if (changed != 16 || !has_vertices(data, 0, 4, 0))
        fail(1, "words changed %u (want 16), the 4 vertices at offset 0 %s", changed,
             has_vertices(data, 0, 4, 0) ? "written" : "not written");
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

/* The filled size belongs to the target, and a swap binds the targets of a
 * state again without setting it as SOSetTargets does, so the last draw goes
 * into A after the first draw's 4 vertices. */
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
    in_a = has_vertices(data_a, 0, 4, 0) && has_vertices(data_a, 4, 8, 1);
    if (!in_a || !has_vertices(data_b, 0, 4, 0) || has_set(data_b, 1))
        fail(1, "the last draw's 8 vertices in A after the first draw's %s, B's own vertices %s, the last draw's "
             "in B %s (want none)",
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

/* The output signature of <code> alone in <blob> of 128 words, a DXBC
 * container with only its OSGN chunk, as D3DGetOutputSignatureBlob returns it
 * (built here with a zero checksum, which DXMT does not check). Returns its
 * size. */
static uint32_t build_signature_blob(const DWORD *code, uint32_t *blob)
{
    const uint32_t *dwords = (const uint32_t *)code;
    uint32_t osgn_size = 0, size, i;
    const char *osgn = NULL;

    for (i = 0; i < dwords[7]; i++)
    {
        const char *chunk = (const char *)code + dwords[8 + i];
        if (!memcmp(chunk, "OSGN", 4))
        {
            osgn = chunk;
            osgn_size = 8 + *(const uint32_t *)(chunk + 4);
        }
    }
    size = 36 + osgn_size;
    if (!osgn || size > 128 * sizeof(*blob)) fail(2, "no output signature to build a blob from");
    memset(blob, 0, 36);
    memcpy(blob, "DXBC", 4);
    blob[5] = 1;
    blob[6] = size;
    blob[7] = 1;
    blob[8] = 36;
    memcpy(&blob[9], osgn, osgn_size);
    return size;
}

/* The bytecode is the output signature of vs_code alone. */
static void signature_blob_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "POSITION", 0, 0, 4, 0 } };
    static uint32_t blob[128];
    static uint32_t data[TARGET_WORDS];
    ID3D11GeometryShader *sig_gs = NULL, *no_decl_gs = NULL, *oversized_gs = NULL;
    HRESULT hr, hr_no_decl, hr_oversized;
    uint32_t size = build_signature_blob(vs_code, blob);
    UINT stride = 16;
    int written = 0;

    hr =ID3D11Device_CreateGeometryShaderWithStreamOutput(device, blob, size, decl, 1, &stride, 1,
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

/* The pixel at x, y of slice <slice> of <texture>, a 64 by 64 R8G8B8A8 render
 * target, as R8G8B8A8 */
static uint32_t read_slice_pixel(ID3D11Texture2D *texture, unsigned slice, unsigned x, unsigned y)
{
    D3D11_TEXTURE2D_DESC desc;
    D3D11_MAPPED_SUBRESOURCE map;
    ID3D11Texture2D *staging;
    uint32_t value;

    ID3D11Texture2D_GetDesc(texture, &desc);
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    check_hr(ID3D11Device_CreateTexture2D(device, &desc, NULL, &staging), "CreateTexture2D");
    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging, (ID3D11Resource *)texture);
    check_hr(ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, slice, D3D11_MAP_READ, 0, &map), "Map");
    value = *(const uint32_t *)((const char *)map.pData + y * map.RowPitch + x * 4);
    ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, slice);
    ID3D11Texture2D_Release(staging);
    return value;
}

/* The pixel of the render target at x, y as R8G8B8A8 */
static uint32_t read_pixel(unsigned x, unsigned y)
{
    return read_slice_pixel(rt, 0, x, y);
}

/* A render target of two slices, which layer_vs and layer_pass_gs draw into
 * slice 1 of, and the view of both */
static void create_layered_rt(ID3D11Texture2D **texture, ID3D11RenderTargetView **view)
{
    D3D11_TEXTURE2D_DESC desc = { 64, 64, 1, 2, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT,
                                  D3D11_BIND_RENDER_TARGET, 0, 0 };

    check_hr(ID3D11Device_CreateTexture2D(device, &desc, NULL, texture), "CreateTexture2D");
    check_hr(ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)*texture, NULL, view),
             "CreateRenderTargetView");
}

#define RED 0xff0000ffu
#define GREEN 0xff00ff00u

/* The positions of a strip of two triangles over the whole render target */
static float quad[4][4];

static float quad_position(unsigned v, unsigned c)
{
    static const float corners[4][2] = { { -1.0f, -1.0f }, { -1.0f, 1.0f }, { 1.0f, -1.0f }, { 1.0f, 1.0f } };

    return c < 2 ? corners[v][c] : c == 2 ? 0.5f : 1.0f;
}

/* Whether the <n> slots of 16 bytes from slot <first> on hold the positions of
 * quad vertices v[0] to v[n - 1] */
static int has_quad(const uint32_t *data, unsigned first, unsigned n, const unsigned *v)
{
    unsigned i, c;

    for (i = 0; i < n; i++)
        for (c = 0; c < 4; c++)
            if (((const float *)data)[(first + i) * 4 + c] != quad_position(v[i], c)) return 0;
    return 1;
}

/* The quad's vertex buffer, input layout and shaders, which bind_quad creates
 * and binds and release_quad releases */
static ID3D11Buffer *quad_vb;
static ID3D11InputLayout *quad_layout;
static ID3D11VertexShader *quad_vs;
static ID3D11PixelShader *quad_ps;

/* Bind the pipeline of setup with the quad, position_vs_code and
 * green_ps_code */
static void bind_quad(void)
{
    static const D3D11_INPUT_ELEMENT_DESC desc[] = {
        { "POSITION", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
    };
    D3D11_BUFFER_DESC vb_desc = { sizeof(quad), D3D11_USAGE_DEFAULT, D3D11_BIND_VERTEX_BUFFER, 0, 0, 0 };
    D3D11_SUBRESOURCE_DATA vb_init = { quad, 0, 0 };
    UINT vb_stride = sizeof(quad[0]), vb_offset = 0;
    unsigned v, c;

    for (v = 0; v < 4; v++)
        for (c = 0; c < 4; c++) quad[v][c] = quad_position(v, c);
    check_hr(ID3D11Device_CreateBuffer(device, &vb_desc, &vb_init, &quad_vb), "CreateBuffer");
    check_hr(ID3D11Device_CreateInputLayout(device, desc, 1, position_vs_code, sizeof(position_vs_code),
                                            &quad_layout),
             "CreateInputLayout");
    check_hr(ID3D11Device_CreateVertexShader(device, position_vs_code, sizeof(position_vs_code), NULL, &quad_vs),
             "CreateVertexShader");
    check_hr(ID3D11Device_CreatePixelShader(device, green_ps_code, sizeof(green_ps_code), NULL, &quad_ps),
             "CreatePixelShader");
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, quad_layout);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &quad_vb, &vb_stride, &vb_offset);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
    ID3D11DeviceContext_VSSetShader(context, quad_vs, NULL, 0);
    ID3D11DeviceContext_PSSetShader(context, quad_ps, NULL, 0);
}

static void release_quad(void)
{
    ID3D11DeviceContext_ClearState(context);
    ID3D11PixelShader_Release(quad_ps);
    ID3D11VertexShader_Release(quad_vs);
    ID3D11InputLayout_Release(quad_layout);
    ID3D11Buffer_Release(quad_vb);
}

/* Clear the render target to red, draw the quad with <quad_gs> into <so> at
 * <offset> (directly or with <args>, which draws it indirectly), and return
 * a pixel the quad covers */
static uint32_t draw_quad(ID3D11GeometryShader *quad_gs, ID3D11Buffer *so, UINT offset, ID3D11Buffer *args)
{
    static const float red[] = { 1.0f, 0.0f, 0.0f, 1.0f };

    ID3D11DeviceContext_ClearRenderTargetView(context, rtv, red);
    ID3D11DeviceContext_GSSetShader(context, quad_gs, NULL, 0);
    bind_target(so, offset);
    if (args)
        ID3D11DeviceContext_DrawInstancedIndirect(context, args, 0);
    else
        ID3D11DeviceContext_Draw(context, 4, 0);
    return read_pixel(10, 20);
}

/* A rasterized stream: the draw is streamed out and drawn, also when the
 * target has no room, since streaming out and rasterizing are independent */
static void rasterized_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "SV_Position", 0, 0, 4, 0 } };
    static const uint32_t arguments[] = { 4, 1, 0, 0 };
    static const unsigned list[] = { 0, 1, 2, 1, 3, 2 };
    static uint32_t blob[128], data[TARGET_WORDS], data_blob[TARGET_WORDS], data_one[TARGET_WORDS];
    UINT stride = 16;
    ID3D11GeometryShader *gs0, *gs_blob, *gs1;
    uint32_t drawn, drawn_full, drawn_indirect, drawn_blob, drawn_one, size;
    ID3D11Buffer *args, *target_blob, *target_one;
    int ok, ok_blob, ok_one;

    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, position_vs_code, sizeof(position_vs_code),
                                                               decl, 1, &stride, 1, 0, NULL, &gs0),
             "CreateGeometryShaderWithStreamOutput with rasterized stream 0");
    size = build_signature_blob(position_vs_code, blob);
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, blob, size, decl, 1, &stride, 1, 0, NULL,
                                                               &gs_blob),
             "CreateGeometryShaderWithStreamOutput from a blob with rasterized stream 0");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, position_vs_code, sizeof(position_vs_code),
                                                               decl, 1, &stride, 1, 1, NULL, &gs1),
             "CreateGeometryShaderWithStreamOutput with rasterized stream 1");
    args = create_data_buffer(arguments, sizeof(arguments), 0, D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS);
    target_blob = create_buffer(TARGET_SIZE, TARGET_FILL);
    target_one = create_buffer(TARGET_SIZE, TARGET_FILL);

    bind_quad();
    drawn = draw_quad(gs0, target, 0, NULL);
    read_buffer(target, data);
    drawn_full = draw_quad(gs0, target, TARGET_SIZE, NULL);
    drawn_indirect = draw_quad(gs0, target, TARGET_SIZE, args);
    drawn_blob = draw_quad(gs_blob, target_blob, 0, NULL);
    read_buffer(target_blob, data_blob);
    drawn_one = draw_quad(gs1, target_one, 0, NULL);
    read_buffer(target_one, data_one);

    release_quad();
    ID3D11Buffer_Release(target_one);
    ID3D11Buffer_Release(target_blob);
    ID3D11Buffer_Release(args);
    ID3D11GeometryShader_Release(gs1);
    ID3D11GeometryShader_Release(gs_blob);
    ID3D11GeometryShader_Release(gs0);
    ok = has_quad(data, 0, ARRAY_SIZE(list), list) && !count_changed(data, 24, TARGET_WORDS, TARGET_FILL);
    ok_blob = has_quad(data_blob, 0, ARRAY_SIZE(list), list) && !count_changed(data_blob, 24, TARGET_WORDS, TARGET_FILL);
    ok_one = has_quad(data_one, 0, ARRAY_SIZE(list), list) && !count_changed(data_one, 24, TARGET_WORDS, TARGET_FILL);
    if (!ok || drawn != GREEN || drawn_full != GREEN || drawn_indirect != GREEN || !ok_blob || drawn_blob != GREEN
        || !ok_one || drawn_one != RED)
        fail(1, "stream 0: the two triangles %s, pixel %#x (want %#x); into a full target: pixel %#x, indirect "
             "%#x; from a blob: the triangles %s, pixel %#x; stream 1: the triangles %s, pixel %#x (want %#x)",
             ok ? "written" : "not written", drawn, GREEN, drawn_full, drawn_indirect,
             ok_blob ? "written" : "not written", drawn_blob, ok_one ? "written" : "not written", drawn_one, RED);
}

/* The DXBC container of an input signature, an output signature and a shader
 * chunk in <out> of CONTAINER_WORDS words, with a zero checksum, which DXMT
 * does not check. Returns its size. */
static uint32_t build_container(uint32_t *out, const uint32_t *isgn, uint32_t isgn_words, const uint32_t *osgn,
                                uint32_t osgn_words, const uint32_t *shdr, uint32_t shdr_words)
{
    const struct
    {
        const uint32_t *words;
        uint32_t count;
    } chunks[] = { { isgn, isgn_words }, { osgn, osgn_words }, { shdr, shdr_words } };
    uint32_t size = 32 + 4 * ARRAY_SIZE(chunks), i;

    memset(out, 0, 32);
    memcpy(out, "DXBC", 4);
    out[5] = 1;
    out[7] = ARRAY_SIZE(chunks);
    for (i = 0; i < ARRAY_SIZE(chunks); i++)
    {
        if (size + 4 * chunks[i].count > CONTAINER_WORDS * sizeof(*out)) fail(2, "a shader does not fit");
        out[8 + i] = size;
        memcpy((char *)out + size, chunks[i].words, 4 * chunks[i].count);
        size += 4 * chunks[i].count;
    }
    out[6] = size;
    return size;
}

static uint32_t build_count_gs(uint32_t *out)
{
    return build_container(out, count_gs_isgn, ARRAY_SIZE(count_gs_isgn), count_gs_osgn, ARRAY_SIZE(count_gs_osgn),
                           count_gs_shdr, ARRAY_SIZE(count_gs_shdr));
}

/* A geometry shader of <shdr> with SV_POSITION in and out */
static uint32_t build_position_gs(uint32_t *out, const uint32_t *shdr, uint32_t shdr_words)
{
    return build_container(out, TRIANGLE_GS_ISGN, TRIANGLE_GS_SIGNATURE_WORDS, TRIANGLE_GS_OSGN,
                           TRIANGLE_GS_SIGNATURE_WORDS, shdr, shdr_words);
}

/* layer_vs_shdr with the input signature of position_vs_code */
static uint32_t build_layer_vs(uint32_t *out)
{
    return build_container(out, (const uint32_t *)position_vs_code + 11, 13, layer_vs_osgn, ARRAY_SIZE(layer_vs_osgn),
                           layer_vs_shdr, ARRAY_SIZE(layer_vs_shdr));
}

static uint32_t build_layer_pass_gs(uint32_t *out)
{
    return build_container(out, layer_gs_isgn, ARRAY_SIZE(layer_gs_isgn), layer_gs_osgn, ARRAY_SIZE(layer_gs_osgn),
                           layer_pass_gs_shdr, ARRAY_SIZE(layer_pass_gs_shdr));
}

/* Appends to <want>, which has <n> points, the points count_gs emits for the
 * primitives 0 to <primitives> - 1 of an instance: i from 0 to (id & 3) - 1
 * for each. Returns the points it then has. */
static unsigned count_points(float (*want)[4], unsigned n, unsigned primitives)
{
    unsigned id, i;

    for (id = 0; id < primitives; id++)
    {
        for (i = 0; i < (id & 3); i++, n++)
        {
            want[n][0] = want[n][2] = want[n][3] = (float)id;
            want[n][1] = (float)i;
        }
    }
    return n;
}

/* Whether the <n> slots of 16 bytes from slot <first> on hold the points
 * <want>[0] to <want>[n - 1] */
static int has_points(const uint32_t *data, unsigned first, const float (*want)[4], unsigned n)
{
    return !memcmp((const float *)data + first * 4, want, n * sizeof(*want));
}

/* A geometry shader that streams out: its primitives are written in the order
 * of the draw, whatever each input primitive emits, and a draw stops at the
 * room of the target */
static void geometry_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "custom", 0, 0, 4, 0 } };
    static uint32_t code[CONTAINER_WORDS], data[TARGET_WORDS];
    static float want[32][4];
    D3D11_QUERY_DATA_SO_STATISTICS statistics;
    ID3D11GeometryShader *count_gs;
    ID3D11VertexShader *id_vs;
    ID3D11Query *query;
    uint32_t size = build_count_gs(code);
    UINT stride = 16;
    unsigned n;
    int ok;

    check_hr(ID3D11Device_CreateVertexShader(device, id_vs_code, sizeof(id_vs_code), NULL, &id_vs),
             "CreateVertexShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, 1, &stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &count_gs),
             "CreateGeometryShaderWithStreamOutput");
    query = create_query(D3D11_QUERY_SO_STATISTICS);
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, NULL);
    ID3D11DeviceContext_VSSetShader(context, id_vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, count_gs, NULL, 0);
    bind_target(target, 0);

    /* 6 points, two instances of 3 points, and 12 points of which the 4 the
       16 slots have room for are written */
    ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)query);
    ID3D11DeviceContext_Draw(context, 5, 0);
    ID3D11DeviceContext_DrawInstanced(context, 3, 2, 0, 0);
    ID3D11DeviceContext_Draw(context, 8, 0);
    ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)query);
    get_query_data(query, &statistics, sizeof(statistics));
    read_buffer(target, data);

    ID3D11DeviceContext_ClearState(context);
    ID3D11Query_Release(query);
    ID3D11GeometryShader_Release(count_gs);
    ID3D11VertexShader_Release(id_vs);
    n = count_points(want, 0, 5);
    n = count_points(want, n, 3);
    n = count_points(want, n, 3);
    n = count_points(want, n, 8);
    ok = n == 24 && has_points(data, 0, (const float (*)[4])want, 16);
    if (!ok || statistics.NumPrimitivesWritten != 16 || statistics.PrimitivesStorageNeeded != 24)
        fail(1, "the points of the three draws %s, primitives written %u (want 16), storage needed %u (want 24)",
             ok ? "in order" : "not in order", (unsigned)statistics.NumPrimitivesWritten,
             (unsigned)statistics.PrimitivesStorageNeeded);
}

/* Triangle strips that a geometry shader cuts are written as lists */
static void geometry_strips_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "SV_Position", 0, 0, 4, 0 } };
    static const unsigned list[] = { 0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2 };
    static uint32_t data[TARGET_WORDS];
    ID3D11GeometryShader *strips_gs;
    UINT stride = 16;
    int ok;

    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, strips_gs_code, sizeof(strips_gs_code), decl,
                                                               1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM, NULL,
                                                               &strips_gs),
             "CreateGeometryShaderWithStreamOutput");
    bind_quad();
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_GSSetShader(context, strips_gs, NULL, 0);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 3, 0);
    read_buffer(target, data);
    release_quad();
    ID3D11GeometryShader_Release(strips_gs);
    ok = has_quad(data, 0, ARRAY_SIZE(list), list);
    if (!ok || count_changed(data, 48, TARGET_WORDS, TARGET_FILL))
        fail(1, "the 4 triangles %s, words after them changed %u (want 0)", ok ? "written" : "not written",
             count_changed(data, 48, TARGET_WORDS, TARGET_FILL));
}

/* A geometry shader with rasterized stream 0 streams out and draws, also into
 * a full target; without a rasterized stream it does not draw */
static void geometry_rasterized_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "so_output", 0, 0, 4, 0 } };
    static const float constant[] = { 0.5f, 0.25f, 0.0f, 1.0f };
    static uint32_t data[TARGET_WORDS], data_off[TARGET_WORDS];
    ID3D11GeometryShader *gs0, *gs_off;
    uint32_t drawn, drawn_full, drawn_off;
    ID3D11Buffer *cb, *target_off;
    UINT stride = 16;
    unsigned correct = 0, v, c;

    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, constant_gs_code, sizeof(constant_gs_code),
                                                               decl, 1, &stride, 1, 0, NULL, &gs0),
             "CreateGeometryShaderWithStreamOutput with rasterized stream 0");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, constant_gs_code, sizeof(constant_gs_code),
                                                               decl, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM,
                                                               NULL, &gs_off),
             "CreateGeometryShaderWithStreamOutput without a rasterized stream");
    cb = create_data_buffer(constant, sizeof(constant), D3D11_BIND_CONSTANT_BUFFER, 0);
    target_off = create_buffer(TARGET_SIZE, TARGET_FILL);
    bind_quad();
    ID3D11DeviceContext_GSSetConstantBuffers(context, 0, 1, &cb);
    drawn = draw_quad(gs0, target, 0, NULL);
    read_buffer(target, data);
    drawn_full = draw_quad(gs0, target, TARGET_SIZE, NULL);
    drawn_off = draw_quad(gs_off, target_off, 0, NULL);
    read_buffer(target_off, data_off);
    release_quad();
    ID3D11Buffer_Release(target_off);
    ID3D11Buffer_Release(cb);
    ID3D11GeometryShader_Release(gs_off);
    ID3D11GeometryShader_Release(gs0);
    for (v = 0; v < 6; v++)
    {
        int same = 1;
        for (c = 0; c < 4; c++)
            same &= ((const float *)data)[v * 4 + c] == constant[c] && ((const float *)data_off)[v * 4 + c] == constant[c];
        correct += same;
    }
    if (correct != 6 || count_changed(data, 24, TARGET_WORDS, TARGET_FILL) || drawn != GREEN || drawn_full != GREEN
        || drawn_off != RED)
        fail(1, "the 6 vertices of the two triangles in both targets %u/6, words after them changed %u (want 0), "
             "pixel %#x, into a full target %#x (want %#x), without a rasterized stream %#x (want %#x)", correct,
             count_changed(data, 24, TARGET_WORDS, TARGET_FILL), drawn, drawn_full, GREEN, drawn_off, RED);
}

/* Four streams into four slots: each stream takes its own points and stops at
 * the room of its own slot, and the SO_STATISTICS_STREAM<n> queries count
 * each stream */
static void geometry_streams_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = {
        { 0, "custom", 0, 0, 4, 0 },
        { 1, "custom", 0, 0, 4, 1 },
        { 2, "custom", 0, 0, 4, 2 },
        { 3, "custom", 0, 0, 4, 3 },
    };
    static const uint32_t counts[] = { 3, 6, 4, 12 };
    static const uint32_t written[] = { 3, 6, 4, 10 };
    static const D3D11_QUERY types[] = {
        D3D11_QUERY_SO_STATISTICS_STREAM0, D3D11_QUERY_SO_STATISTICS_STREAM1,
        D3D11_QUERY_SO_STATISTICS_STREAM2, D3D11_QUERY_SO_STATISTICS_STREAM3,
    };
    /* Slot 3 has room for 10 points */
    UINT offsets[] = { 0, 0, 0, TARGET_SIZE - 160 };
    static uint32_t data[4][TARGET_WORDS];
    D3D11_QUERY_DATA_SO_STATISTICS statistics[4];
    ID3D11GeometryShader *streams_gs;
    ID3D11Buffer *targets[4], *cb;
    ID3D11VertexShader *id_vs;
    ID3D11Query *queries[4];
    unsigned s, v, c, correct = 0, counted = 0;

    check_hr(ID3D11Device_CreateVertexShader(device, id_vs_code, sizeof(id_vs_code), NULL, &id_vs),
             "CreateVertexShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, streams_gs_code, sizeof(streams_gs_code), decl,
                                                               4, NULL, 0, D3D11_SO_NO_RASTERIZED_STREAM, NULL,
                                                               &streams_gs),
             "CreateGeometryShaderWithStreamOutput");
    cb = create_data_buffer(counts, sizeof(counts), D3D11_BIND_CONSTANT_BUFFER, 0);
    targets[0] = target;
    for (s = 1; s < 4; s++) targets[s] = create_buffer(TARGET_SIZE, TARGET_FILL);
    for (s = 0; s < 4; s++) queries[s] = create_query(types[s]);
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, NULL);
    ID3D11DeviceContext_VSSetShader(context, id_vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, streams_gs, NULL, 0);
    ID3D11DeviceContext_GSSetConstantBuffers(context, 0, 1, &cb);
    ID3D11DeviceContext_SOSetTargets(context, 4, targets, offsets);
    for (s = 0; s < 4; s++) ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)queries[s]);
    ID3D11DeviceContext_Draw(context, 16, 0);
    for (s = 0; s < 4; s++) ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)queries[s]);
    for (s = 0; s < 4; s++)
    {
        get_query_data(queries[s], &statistics[s], sizeof(statistics[s]));
        read_buffer(targets[s], data[s]);
    }
    ID3D11DeviceContext_ClearState(context);
    for (s = 0; s < 4; s++)
    {
        unsigned first = offsets[s] / 16;
        int same = 1;

        for (v = 0; v < written[s]; v++)
            for (c = 0; c < 4; c++) same &= ((const float *)data[s])[(first + v) * 4 + c] == (float)v;
        correct += same && count_changed(data[s], 0, first * 4, TARGET_FILL) == 0
                   && count_changed(data[s], (first + written[s]) * 4, TARGET_WORDS, TARGET_FILL) == 0;
        counted += statistics[s].NumPrimitivesWritten == written[s] && statistics[s].PrimitivesStorageNeeded == counts[s];
        ID3D11Query_Release(queries[s]);
        if (s) ID3D11Buffer_Release(targets[s]);
    }
    ID3D11Buffer_Release(cb);
    ID3D11GeometryShader_Release(streams_gs);
    ID3D11VertexShader_Release(id_vs);
    if (correct != 4 || counted != 4)
        fail(1, "streams with their points in their slots %u/4, with the counts of the queries %u/4; stream 3: "
             "written %u (want 10), needed %u (want 12)", correct, counted,
             (unsigned)statistics[3].NumPrimitivesWritten, (unsigned)statistics[3].PrimitivesStorageNeeded);
}

/* A pixel shader that returns the COLOR it reads from register <reg>,
 * assembled here as ps_4_0 with the OSGN chunk of green_ps_code */
static uint32_t build_color_ps(uint32_t *out, uint32_t reg)
{
    const uint32_t isgn[] = {
        0x4e475349, 0x00000028, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003,
        reg, 0x00000f0f, 0x4f4c4f43, 0xabab0052,
    };
    const uint32_t shdr[] = {
        0x52444853, 0x00000038, 0x00000040, 0x0000000e,
        0x03001062, 0x001010f2, reg,                                /* dcl_input_ps linear v<reg>.xyzw */
        0x03000065, 0x001020f2, 0x00000000,                         /* dcl_output o0.xyzw */
        0x05000036, 0x001020f2, 0x00000000, 0x00101e46, reg,        /* mov o0, v<reg> */
        0x0100003e,                                                 /* ret */
    };

    return build_container(out, isgn, ARRAY_SIZE(isgn), (const uint32_t *)green_ps_code + 24, 13, shdr,
                           ARRAY_SIZE(shdr));
}

/* The four streams of raster_streams_shdr, one point each, with rasterized
 * stream 0, 1, 2 and 3 in turn: only the rasterized stream's point is drawn,
 * in its colour, and each draw appends every stream's point to its slot */
static void geometry_streams_rasterized_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = {
        { 0, "COLOR", 0, 0, 4, 0 },
        { 1, "COLOR", 0, 0, 4, 1 },
        { 2, "COLOR", 0, 0, 4, 2 },
        { 3, "COLOR", 0, 0, 4, 3 },
    };
    static const float colors[4][4] = {
        { 0.0f, 1.0f, 0.0f, 1.0f }, { 0.0f, 0.0f, 1.0f, 1.0f }, { 1.0f, 1.0f, 1.0f, 1.0f }, { 1.0f, 1.0f, 0.0f, 1.0f },
    };
    static const uint32_t pixel_colors[4] = { 0xff00ff00u, 0xffff0000u, 0xffffffffu, 0xff00ffffu };
    static const float clear[] = { 0.0f, 0.0f, 0.0f, 0.0f };
    static uint32_t gs_code[CONTAINER_WORDS], ps_code[CONTAINER_WORDS], data[4][TARGET_WORDS];
    ID3D11GeometryShader *streams_gs;
    ID3D11PixelShader *color_ps;
    ID3D11VertexShader *id_vs;
    ID3D11Buffer *targets[4];
    UINT offsets[4];
    uint32_t pixels[4], gs_size, ps_size;
    unsigned r, s, correct = 0;

    check_hr(ID3D11Device_CreateVertexShader(device, id_vs_code, sizeof(id_vs_code), NULL, &id_vs),
             "CreateVertexShader");
    gs_size = build_container(gs_code, count_gs_isgn, ARRAY_SIZE(count_gs_isgn), raster_streams_osg5,
                              ARRAY_SIZE(raster_streams_osg5), raster_streams_shdr, ARRAY_SIZE(raster_streams_shdr));
    targets[0] = target;
    for (s = 1; s < 4; s++) targets[s] = create_buffer(TARGET_SIZE, TARGET_FILL);
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, NULL);
    ID3D11DeviceContext_VSSetShader(context, id_vs, NULL, 0);
    for (r = 0; r < 4; r++)
    {
        int ok = 1;

        check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, gs_code, gs_size, decl, 4, NULL, 0, r,
                                                                   NULL, &streams_gs),
                 "CreateGeometryShaderWithStreamOutput");
        ps_size = build_color_ps(ps_code, 1 + r);
        check_hr(ID3D11Device_CreatePixelShader(device, ps_code, ps_size, NULL, &color_ps), "CreatePixelShader");
        /* The first draw writes at the start of each slot, each later one
           appends its point */
        for (s = 0; s < 4; s++) offsets[s] = r ? ~0u : 0;
        ID3D11DeviceContext_ClearRenderTargetView(context, rtv, clear);
        ID3D11DeviceContext_GSSetShader(context, streams_gs, NULL, 0);
        ID3D11DeviceContext_PSSetShader(context, color_ps, NULL, 0);
        ID3D11DeviceContext_SOSetTargets(context, 4, targets, offsets);
        ID3D11DeviceContext_Draw(context, 1, 0);
        for (s = 0; s < 4; s++)
        {
            pixels[s] = read_pixel(8 + 16 * s, 32);
            read_buffer(targets[s], data[s]);
        }
        ID3D11PixelShader_Release(color_ps);
        ID3D11GeometryShader_Release(streams_gs);
        say("rasterized stream %u: pixels %#x %#x %#x %#x", r, pixels[0], pixels[1], pixels[2], pixels[3]);
        for (s = 0; s < 4; s++)
        {
            const float *point = (const float *)data[s] + 4 * r;
            unsigned changed = count_changed(data[s], 4 * (r + 1), TARGET_WORDS, TARGET_FILL);

            say("rasterized stream %u: stream %u point %u (%g, %g, %g, %g), words after it changed %u", r, s, r,
                point[0], point[1], point[2], point[3], changed);
            ok &= pixels[s] == (s == r ? pixel_colors[r] : 0u);
            ok &= !memcmp(point, colors[s], sizeof(colors[s])) && !changed;
        }
        correct += ok;
    }
    ID3D11DeviceContext_ClearState(context);
    for (s = 1; s < 4; s++) ID3D11Buffer_Release(targets[s]);
    ID3D11VertexShader_Release(id_vs);
    if (correct != 4)
        fail(1, "rasterized streams that drew only their point in their colour and appended every stream's point "
             "%u/4", correct);
}

#define UAV_POINTS 40

/* Draws UAV_POINTS points, more than an object threadgroup of 32 takes,
 * through <vertex> and <geometry>, which streams out into a target of
 * LARGE_SIZE bytes, with a raw UAV of 4 zeroed words at u1 of the output
 * merger. Returns the UAV's first word after the draw, and the target in
 * <data>. */
static uint32_t draw_uav_points(ID3D11VertexShader *vertex, ID3D11GeometryShader *geometry, uint32_t *data)
{
    static const uint32_t zero[4] = { 0 };
    D3D11_UNORDERED_ACCESS_VIEW_DESC uav_desc;
    ID3D11UnorderedAccessView *uav;
    ID3D11Buffer *buffer, *large;
    uint32_t counter[4];

    buffer = create_data_buffer(zero, sizeof(zero), D3D11_BIND_UNORDERED_ACCESS,
                                D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS);
    memset(&uav_desc, 0, sizeof(uav_desc));
    uav_desc.Format = DXGI_FORMAT_R32_TYPELESS;
    uav_desc.ViewDimension = D3D11_UAV_DIMENSION_BUFFER;
    uav_desc.Buffer.NumElements = ARRAY_SIZE(zero);
    uav_desc.Buffer.Flags = D3D11_BUFFER_UAV_FLAG_RAW;
    check_hr(ID3D11Device_CreateUnorderedAccessView(device, (ID3D11Resource *)buffer, &uav_desc, &uav),
             "CreateUnorderedAccessView");
    large = create_buffer(LARGE_SIZE, TARGET_FILL);
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, NULL);
    ID3D11DeviceContext_VSSetShader(context, vertex, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, geometry, NULL, 0);
    ID3D11DeviceContext_OMSetRenderTargetsAndUnorderedAccessViews(context, 1, &rtv, NULL, 1, 1, &uav, NULL);
    bind_target(large, 0);
    ID3D11DeviceContext_Draw(context, UAV_POINTS, 0);
    read_buffer_size(buffer, counter, sizeof(counter));
    read_buffer_size(large, data, LARGE_SIZE);

    ID3D11DeviceContext_ClearState(context);
    ID3D11Buffer_Release(large);
    ID3D11UnorderedAccessView_Release(uav);
    ID3D11Buffer_Release(buffer);
    return counter[0];
}

/* A geometry shader that streams out and adds 1 to a UAV for each input
 * point: after a draw of 40 points the UAV holds 40, and the target the 40
 * points in order */
static void geometry_uav_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "custom", 0, 0, 4, 0 } };
    static uint32_t code[CONTAINER_WORDS], data[LARGE_WORDS];
    ID3D11GeometryShader *uav_gs;
    ID3D11VertexShader *id_vs;
    UINT stride = 16;
    uint32_t size, counter;
    unsigned i, c, correct = 0;

    size = build_container(code, count_gs_isgn, ARRAY_SIZE(count_gs_isgn), count_gs_osgn, ARRAY_SIZE(count_gs_osgn),
                           uav_gs_shdr, ARRAY_SIZE(uav_gs_shdr));
    check_hr(ID3D11Device_CreateVertexShader(device, id_vs_code, sizeof(id_vs_code), NULL, &id_vs),
             "CreateVertexShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, 1, &stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &uav_gs),
             "CreateGeometryShaderWithStreamOutput");
    counter = draw_uav_points(id_vs, uav_gs, data);
    ID3D11GeometryShader_Release(uav_gs);
    ID3D11VertexShader_Release(id_vs);
    for (i = 0; i < UAV_POINTS; i++)
    {
        int same = 1;

        for (c = 0; c < 4; c++) same &= ((const float *)data)[i * 4 + c] == (float)i;
        correct += same;
    }
    say("UAV %u, points in order %u/%u", counter, correct, UAV_POINTS);
    if (counter != UAV_POINTS || correct != UAV_POINTS || count_changed(data, 4 * UAV_POINTS, LARGE_WORDS, TARGET_FILL))
        fail(1, "UAV %u (want %u), points in order %u/%u, words after them changed %u (want 0)", counter, UAV_POINTS,
             correct, UAV_POINTS, count_changed(data, 4 * UAV_POINTS, LARGE_WORDS, TARGET_FILL));
}

/* A vertex shader that adds 1 to a UAV for each vertex, through count_gs
 * streaming out: after a draw of 40 points the UAV holds 40, and the target
 * the points count_gs emits in order */
static void geometry_vs_uav_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "custom", 0, 0, 4, 0 } };
    static uint32_t code[CONTAINER_WORDS], data[LARGE_WORDS];
    static float want[128][4];
    ID3D11GeometryShader *count_gs;
    ID3D11VertexShader *uav_vs;
    UINT stride = 16;
    uint32_t size, counter;
    unsigned n;
    int ok;

    size = build_container(code, (const uint32_t *)id_vs_code + 11, 13, (const uint32_t *)id_vs_code + 24, 12,
                           uav_vs_shdr, ARRAY_SIZE(uav_vs_shdr));
    check_hr(ID3D11Device_CreateVertexShader(device, code, size, NULL, &uav_vs), "CreateVertexShader");
    size = build_count_gs(code);
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, 1, &stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &count_gs),
             "CreateGeometryShaderWithStreamOutput");
    counter = draw_uav_points(uav_vs, count_gs, data);
    ID3D11GeometryShader_Release(count_gs);
    ID3D11VertexShader_Release(uav_vs);
    n = count_points(want, 0, UAV_POINTS);
    ok = has_points(data, 0, (const float (*)[4])want, n) && !count_changed(data, 4 * n, LARGE_WORDS, TARGET_FILL);
    say("UAV %u, the %u points %s", counter, n, ok ? "in order" : "not in order");
    if (counter != UAV_POINTS || !ok)
        fail(1, "UAV %u (want %u), the %u points %s", counter, UAV_POINTS, n,
             ok ? "in order" : "not in order or words after them changed");
}

/* Bind id_vs_code and count_gs streaming out into <so> at offset 0, and
 * return the shaders in <vs_out> and <gs_out> */
static void bind_count_gs(ID3D11Buffer *so, ID3D11VertexShader **vs_out, ID3D11GeometryShader **gs_out)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "custom", 0, 0, 4, 0 } };
    static uint32_t code[CONTAINER_WORDS];
    uint32_t size = build_count_gs(code);
    UINT stride = 16;

    check_hr(ID3D11Device_CreateVertexShader(device, id_vs_code, sizeof(id_vs_code), NULL, vs_out),
             "CreateVertexShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, 1, &stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, gs_out),
             "CreateGeometryShaderWithStreamOutput");
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, NULL);
    ID3D11DeviceContext_VSSetShader(context, *vs_out, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, *gs_out, NULL, 0);
    bind_target(so, 0);
}

/* Two instances of 40 points, more than an object threadgroup of 32 takes:
 * the points are written in the order of the draw */
static void geometry_order_case(void)
{
    static uint32_t data[LARGE_WORDS];
    static float want[128][4];
    ID3D11GeometryShader *count_gs;
    ID3D11VertexShader *id_vs;
    ID3D11Buffer *large;
    unsigned n;
    int ok;

    large = create_buffer(LARGE_SIZE, TARGET_FILL);
    bind_count_gs(large, &id_vs, &count_gs);
    ID3D11DeviceContext_DrawInstanced(context, 40, 2, 0, 0);
    read_buffer_size(large, data, LARGE_SIZE);
    ID3D11DeviceContext_ClearState(context);
    ID3D11Buffer_Release(large);
    ID3D11GeometryShader_Release(count_gs);
    ID3D11VertexShader_Release(id_vs);
    n = count_points(want, 0, 40);
    n = count_points(want, n, 40);
    ok = has_points(data, 0, (const float (*)[4])want, n);
    if (!ok || count_changed(data, n * 4, LARGE_WORDS, TARGET_FILL))
        fail(1, "the %u points %s, words after them changed %u (want 0)", n, ok ? "in order" : "not in order",
             count_changed(data, n * 4, LARGE_WORDS, TARGET_FILL));
}

#define LARGE_DRAW 64000
#define LARGE_DRAW_POINTS (LARGE_DRAW / 4 * 6)

/* A draw of 64000 points through that shader, whose 96000 points are
 * written in order and counted by SO_STATISTICS: as many counts as DXMT once
 * added up in one GPU thread, for which Metal reported a GPU hang */
static void geometry_large_case(void)
{
    static uint32_t data[LARGE_DRAW_POINTS * 4];
    static float want[LARGE_DRAW_POINTS][4];
    D3D11_BUFFER_DESC desc = { sizeof(data), D3D11_USAGE_DEFAULT, D3D11_BIND_STREAM_OUTPUT, 0, 0, 0 };
    D3D11_QUERY_DATA_SO_STATISTICS statistics;
    ID3D11GeometryShader *count_gs;
    ID3D11VertexShader *id_vs;
    ID3D11Buffer *huge;
    ID3D11Query *query;
    unsigned n;
    int ok;

    check_hr(ID3D11Device_CreateBuffer(device, &desc, NULL, &huge), "CreateBuffer");
    query = create_query(D3D11_QUERY_SO_STATISTICS);
    bind_count_gs(huge, &id_vs, &count_gs);
    ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)query);
    ID3D11DeviceContext_Draw(context, LARGE_DRAW, 0);
    ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)query);
    get_query_data(query, &statistics, sizeof(statistics));
    read_buffer_size(huge, data, sizeof(data));
    ID3D11DeviceContext_ClearState(context);
    ID3D11Query_Release(query);
    ID3D11Buffer_Release(huge);
    ID3D11GeometryShader_Release(count_gs);
    ID3D11VertexShader_Release(id_vs);
    n = count_points(want, 0, LARGE_DRAW);
    ok = n == LARGE_DRAW_POINTS && has_points(data, 0, (const float (*)[4])want, n);
    if (!ok || statistics.NumPrimitivesWritten != n || statistics.PrimitivesStorageNeeded != n)
        fail(1, "the %u points %s, primitives written %u, storage needed %u", n, ok ? "in order" : "not in order",
             (unsigned)statistics.NumPrimitivesWritten, (unsigned)statistics.PrimitivesStorageNeeded);
}

/* Indirect draws through a geometry shader that streams out append like
 * direct ones. One with more than the 65536 mesh threadgroups DXMT has room
 * for is not drawn, which DXMT logs once at the indirect draw after it. */
static void geometry_indirect_case(void)
{
    static const uint32_t arguments[3][4] = { { 5, 1, 0, 0 }, { 65568, 1, 0, 0 }, { 1, 1, 0, 0 } };
    static uint32_t data[TARGET_WORDS];
    static float want[16][4];
    ID3D11GeometryShader *count_gs;
    ID3D11VertexShader *id_vs;
    ID3D11Buffer *args[3];
    unsigned i, n;
    int ok;

    for (i = 0; i < 3; i++)
        args[i] = create_data_buffer(arguments[i], sizeof(arguments[i]), 0, D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS);
    bind_count_gs(target, &id_vs, &count_gs);
    ID3D11DeviceContext_DrawInstancedIndirect(context, args[0], 0);
    ID3D11DeviceContext_DrawInstancedIndirect(context, args[1], 0);
    ID3D11DeviceContext_DrawInstancedIndirect(context, args[0], 0);
    read_buffer(target, data);
    /* This draw emits nothing. The readback waits for it: a process that
       releases the device right after a draw crashes in DXMT (#158). */
    ID3D11DeviceContext_DrawInstancedIndirect(context, args[2], 0);
    read_buffer(target, data);
    ID3D11DeviceContext_ClearState(context);
    for (i = 0; i < 3; i++) ID3D11Buffer_Release(args[i]);
    ID3D11GeometryShader_Release(count_gs);
    ID3D11VertexShader_Release(id_vs);
    n = count_points(want, 0, 5);
    n = count_points(want, n, 5);
    ok = has_points(data, 0, (const float (*)[4])want, n);
    if (!ok || count_changed(data, n * 4, TARGET_WORDS, TARGET_FILL))
        fail(1, "the %u points of the two draws %s, words after them changed %u (want 0)", n,
             ok ? "in order" : "not in order", count_changed(data, n * 4, TARGET_WORDS, TARGET_FILL));
}

/* Draw the quad's first triangle (vertices 0 to 2) through <code> streaming
 * out <decl> with <stride> into a target of LARGE_SIZE, which <data> gets */
static void draw_triangle_gs(const void *code, SIZE_T size, const D3D11_SO_DECLARATION_ENTRY *decl, UINT entries,
                             UINT stride, uint32_t *data)
{
    ID3D11GeometryShader *shape_gs;
    ID3D11Buffer *large;

    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, entries, &stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &shape_gs),
             "CreateGeometryShaderWithStreamOutput");
    large = create_buffer(LARGE_SIZE, TARGET_FILL);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_GSSetShader(context, shape_gs, NULL, 0);
    bind_target(large, 0);
    ID3D11DeviceContext_Draw(context, 3, 0);
    read_buffer_size(large, data, LARGE_SIZE);
    ID3D11Buffer_Release(large);
    ID3D11GeometryShader_Release(shape_gs);
}

/* What a geometry shader emits is written as lists: a triangle strip of 4
 * vertices without a cut, whose second triangle takes the order of an odd
 * one; a line strip of 3 vertices; and the triangle of each of 4 geometry
 * shader instances, in instance order */
static void geometry_shapes_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "SV_Position", 0, 0, 4, 0 } };
    static const D3D11_SO_DECLARATION_ENTRY instances_decl[] = {
        { 0, "SV_Position", 0, 0, 4, 0 },
        { 0, "SV_RenderTargetArrayIndex", 0, 0, 1, 0 },
    };
    static const unsigned strip[] = { 0, 1, 2, 1, 0, 2 }, lines[] = { 0, 1, 1, 2 };
    static const uint32_t layer_offset[4] = { 0 };
    static uint32_t code[CONTAINER_WORDS], strip_data[LARGE_WORDS], line_data[LARGE_WORDS], instance_data[LARGE_WORDS];
    unsigned instance, v, c, correct = 0;
    ID3D11Buffer *cb;
    uint32_t size;
    int ok_strip, ok_lines;

    bind_quad();
    size = build_position_gs(code, strip4_gs_shdr, ARRAY_SIZE(strip4_gs_shdr));
    draw_triangle_gs(code, size, decl, 1, 16, strip_data);
    size = build_position_gs(code, line_gs_shdr, ARRAY_SIZE(line_gs_shdr));
    draw_triangle_gs(code, size, decl, 1, 16, line_data);
    cb = create_data_buffer(layer_offset, sizeof(layer_offset), D3D11_BIND_CONSTANT_BUFFER, 0);
    ID3D11DeviceContext_GSSetConstantBuffers(context, 0, 1, &cb);
    draw_triangle_gs(instances_gs_code, sizeof(instances_gs_code), instances_decl, 2, 20, instance_data);
    release_quad();
    ID3D11Buffer_Release(cb);

    ok_strip = has_quad(strip_data, 0, ARRAY_SIZE(strip), strip)
               && !count_changed(strip_data, 4 * ARRAY_SIZE(strip), LARGE_WORDS, TARGET_FILL);
    ok_lines = has_quad(line_data, 0, ARRAY_SIZE(lines), lines)
               && !count_changed(line_data, 4 * ARRAY_SIZE(lines), LARGE_WORDS, TARGET_FILL);
    for (instance = 0; instance < 4; instance++)
    {
        for (v = 0; v < 3; v++)
        {
            const uint32_t *vertex = &instance_data[(instance * 3 + v) * 5];
            int same = vertex[4] == instance;

            for (c = 0; c < 4; c++) same &= ((const float *)vertex)[c] == quad_position(v, c);
            correct += same;
        }
    }
    if (count_changed(instance_data, 60, LARGE_WORDS, TARGET_FILL)) correct = 0;
    if (!ok_strip || !ok_lines || correct != 12)
        fail(1, "the uncut strip %s, the line strip %s, the vertices of the 4 instances %u/12",
             ok_strip ? "written" : "not written", ok_lines ? "written" : "not written", correct);
}

/* An indexed triangle strip with a cut index at an even position between two
 * quads: the cut leaves a gap between them, and the strip after it starts
 * with the winding of a first triangle, so the default rasterizer state draws
 * the right quad. Every pipeline draws both quads: a geometry shader that
 * passes its triangles and layer through (layer_pass_gs after layer_vs, into
 * slice 1 of a render target of two, slice 0 left alone) and one that does
 * not pass its triangles through, each with and without stream output, no
 * geometry shader, and a vertex shader that streams out. The stream output
 * of each writes the 4 triangles of the quads as a list, the triangles that
 * hold the cut left out, and SO_STATISTICS counts 4. */
static void geometry_cut_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "SV_Position", 0, 0, 4, 0 } };
    static const char *names[] = { "layered pass-through", "layered pass-through without stream output", "loop",
                                   "loop without stream output", "no geometry shader", "vertex shader" };
    static const float red[] = { 1.0f, 0.0f, 0.0f, 1.0f };
    static const float vertices[8][4] = {
        { -1.0f, -1.0f, 0.5f, 1.0f }, { -1.0f, 1.0f, 0.5f, 1.0f },
        { -0.25f, -1.0f, 0.5f, 1.0f }, { -0.25f, 1.0f, 0.5f, 1.0f },
        { 0.25f, -1.0f, 0.5f, 1.0f }, { 0.25f, 1.0f, 0.5f, 1.0f },
        { 1.0f, -1.0f, 0.5f, 1.0f }, { 1.0f, 1.0f, 0.5f, 1.0f },
    };
    static const uint16_t indices[] = { 0, 1, 2, 3, 0xffff, 4, 5, 6, 7 };
    static const unsigned list[] = { 0, 1, 2, 1, 3, 2, 4, 5, 6, 5, 7, 6 };
    static uint32_t code[CONTAINER_WORDS], vs_code_layer[CONTAINER_WORDS], data[6][TARGET_WORDS];
    ID3D11GeometryShader *shaders[6] = { NULL, NULL, NULL, NULL, NULL, NULL };
    D3D11_QUERY_DATA_SO_STATISTICS statistics[6];
    ID3D11Buffer *vb, *ib, *targets[6] = { NULL, NULL, NULL, NULL, NULL, NULL };
    ID3D11Query *queries[6] = { NULL, NULL, NULL, NULL, NULL, NULL };
    ID3D11RenderTargetView *layered_rtv;
    ID3D11Texture2D *layered_rt;
    ID3D11VertexShader *layer_vs;
    UINT stride = 16, offset = 0;
    uint32_t pixels[6][4], size;
    unsigned i, v, c, failed = 0;

    size = build_layer_vs(vs_code_layer);
    check_hr(ID3D11Device_CreateVertexShader(device, vs_code_layer, size, NULL, &layer_vs), "CreateVertexShader");
    size = build_layer_pass_gs(code);
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, 1, &stride, 1, 0, NULL,
                                                               &shaders[0]),
             "CreateGeometryShaderWithStreamOutput of a pass-through shader");
    check_hr(ID3D11Device_CreateGeometryShader(device, code, size, NULL, &shaders[1]), "CreateGeometryShader");
    create_layered_rt(&layered_rt, &layered_rtv);
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, triangle_gs_code, sizeof(triangle_gs_code),
                                                               decl, 1, &stride, 1, 0, NULL, &shaders[2]),
             "CreateGeometryShaderWithStreamOutput");
    check_hr(ID3D11Device_CreateGeometryShader(device, triangle_gs_code, sizeof(triangle_gs_code), NULL, &shaders[3]),
             "CreateGeometryShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, position_vs_code, sizeof(position_vs_code),
                                                               decl, 1, &stride, 1, 0, NULL, &shaders[5]),
             "CreateGeometryShaderWithStreamOutput of a vertex shader");
    vb = create_data_buffer(vertices, sizeof(vertices), D3D11_BIND_VERTEX_BUFFER, 0);
    ib = create_data_buffer(indices, sizeof(indices), D3D11_BIND_INDEX_BUFFER, 0);
    bind_quad();
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &stride, &offset);
    ID3D11DeviceContext_IASetIndexBuffer(context, ib, DXGI_FORMAT_R16_UINT, 0);
    for (i = 0; i < ARRAY_SIZE(shaders); i++)
    {
        int streams = i == 0 || i == 2 || i == 5, layered = i < 2;

        ID3D11DeviceContext_OMSetRenderTargets(context, 1, layered ? &layered_rtv : &rtv, NULL);
        ID3D11DeviceContext_ClearRenderTargetView(context, layered ? layered_rtv : rtv, red);
        ID3D11DeviceContext_VSSetShader(context, layered ? layer_vs : quad_vs, NULL, 0);
        ID3D11DeviceContext_GSSetShader(context, shaders[i], NULL, 0);
        if (streams)
        {
            targets[i] = create_buffer(TARGET_SIZE, TARGET_FILL);
            queries[i] = create_query(D3D11_QUERY_SO_STATISTICS);
            bind_target(targets[i], 0);
            ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)queries[i]);
        }
        ID3D11DeviceContext_DrawIndexed(context, ARRAY_SIZE(indices), 0, 0);
        if (streams)
        {
            ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)queries[i]);
            get_query_data(queries[i], &statistics[i], sizeof(statistics[i]));
            read_buffer(targets[i], data[i]);
        }
        /* The quads, in slice 1 of the layered render target, and a pixel of
           the left quad in its slice 0, which nothing draws to */
        pixels[i][0] = read_slice_pixel(layered ? layered_rt : rt, layered, 10, 32);
        pixels[i][1] = read_slice_pixel(layered ? layered_rt : rt, layered, 32, 32);
        pixels[i][2] = read_slice_pixel(layered ? layered_rt : rt, layered, 54, 32);
        pixels[i][3] = layered ? read_slice_pixel(layered_rt, 0, 10, 32) : RED;
    }
    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &rtv, NULL);
    release_quad();
    ID3D11RenderTargetView_Release(layered_rtv);
    ID3D11Texture2D_Release(layered_rt);
    ID3D11VertexShader_Release(layer_vs);
    ID3D11Buffer_Release(ib);
    ID3D11Buffer_Release(vb);
    for (i = 0; i < ARRAY_SIZE(shaders); i++)
    {
        if (shaders[i]) ID3D11GeometryShader_Release(shaders[i]);
        if (targets[i]) ID3D11Buffer_Release(targets[i]);
        if (queries[i]) ID3D11Query_Release(queries[i]);
    }
    for (i = 0; i < ARRAY_SIZE(shaders); i++)
    {
        int written = 1;

        if (targets[i])
        {
            for (v = 0; v < ARRAY_SIZE(list); v++)
                for (c = 0; c < 4; c++)
                    written &= ((const float *)data[i])[v * 4 + c] == vertices[list[v]][c];
            written &= !count_changed(data[i], 4 * ARRAY_SIZE(list), TARGET_WORDS, TARGET_FILL);
            say("%s: left %#x, gap %#x, right %#x, slice 0 %#x; the 4 triangles %s, primitives written %u, "
                "storage needed %u", names[i], pixels[i][0], pixels[i][1], pixels[i][2], pixels[i][3],
                written ? "written" : "not written", (unsigned)statistics[i].NumPrimitivesWritten,
                (unsigned)statistics[i].PrimitivesStorageNeeded);
            written &= statistics[i].NumPrimitivesWritten == 4 && statistics[i].PrimitivesStorageNeeded == 4;
        }
        else
        {
            say("%s: left %#x, gap %#x, right %#x, slice 0 %#x", names[i], pixels[i][0], pixels[i][1], pixels[i][2],
                pixels[i][3]);
        }
        if (pixels[i][0] != GREEN || pixels[i][1] != RED || pixels[i][2] != GREEN || pixels[i][3] != RED || !written)
            failed++;
    }
    if (failed)
        fail(1, "%u pipelines (want 0) did not draw %#x, %#x, %#x into their slice or did not stream out the 4 "
             "triangles", failed, GREEN, RED, GREEN);
}

/* Positions of the indexed strip of geometry-cut-blocks: the strip runs over
 * blocks of DXMT's strip cut scan (1024 indices), with cuts at even and odd
 * positions, so that a strip crosses a block with the winding of a cut in
 * the block before it */
#define CUT_BLOCKS_COUNT 2600
#define CUT_BLOCKS_START 3
#define CUT_BLOCKS_INSTANCES 2
#define CUT_BLOCKS_VERTICES 4096
#define CUT_BLOCKS_TARGET (CUT_BLOCKS_COUNT * CUT_BLOCKS_INSTANCES * 3 * 16)

static int is_cut_position(unsigned p)
{
    static const unsigned cuts[] = { 0, 4, 6, 1500, 2101, 2102, 2104, CUT_BLOCKS_COUNT - 1 };
    unsigned i;

    for (i = 0; i < ARRAY_SIZE(cuts); i++)
        if (p == cuts[i]) return 1;
    return 0;
}

/* The vertices D3D11 streams out of the strip, <vertices> of each primitive
 * of a strip whose primitive takes <vertices> indices, and with the winding
 * of each triangle's place in its strip. Returns the primitives of an
 * instance. */
static unsigned cut_blocks_primitives(unsigned vertices, uint16_t *out)
{
    unsigned p, v, first = 0, primitives = 0;

    for (p = 0; p + vertices <= CUT_BLOCKS_COUNT; p++)
    {
        int cut = 0;

        for (v = 0; v < vertices; v++) cut |= is_cut_position(p + v);
        if (is_cut_position(p)) first = p + 1;
        if (cut) continue;
        for (v = 0; v < vertices; v++) out[primitives * vertices + v] = p + v;
        if (vertices == 3 && ((p - first) & 1))
        {
            out[primitives * 3 + 1] = p + 2;
            out[primitives * 3 + 2] = p + 1;
        }
        primitives++;
    }
    return primitives;
}

/* An indexed strip of 2600 indices with cut indices, the first and the last
 * among them, of 16 and of 32 bits, drawn with a start index and two
 * instances, directly and indirectly: through a geometry shader that does
 * not pass its triangles through (DXMT's geometry pipeline), and through a
 * vertex shader that streams out (DXMT's emulated stream output), also as a
 * line strip. Each instance writes the primitives without a cut in the order
 * of the strip, each triangle with the winding of its place in its strip, and
 * SO_STATISTICS counts them. */
static void geometry_cut_blocks_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "SV_Position", 0, 0, 4, 0 } };
    static const D3D11_INPUT_ELEMENT_DESC layout_desc[] = {
        { "POSITION", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
    };
    static const char *names[] = { "geometry shader", "geometry shader, indirect", "vertex shader",
                                   "vertex shader, indirect", "vertex shader, line strip" };
    static const uint32_t arguments[] = { CUT_BLOCKS_COUNT, CUT_BLOCKS_INSTANCES, CUT_BLOCKS_START, 0, 0 };
    static const char *formats[] = { "16-bit", "32-bit" };
    static float vertices[CUT_BLOCKS_VERTICES][4];
    static uint16_t indices16[CUT_BLOCKS_START + CUT_BLOCKS_COUNT], want[CUT_BLOCKS_COUNT * 3];
    static uint32_t indices32[CUT_BLOCKS_START + CUT_BLOCKS_COUNT], data[CUT_BLOCKS_TARGET / 4];
    D3D11_BUFFER_DESC target_desc = { CUT_BLOCKS_TARGET, D3D11_USAGE_DEFAULT, D3D11_BIND_STREAM_OUTPUT, 0, 0, 0 };
    D3D11_QUERY_DATA_SO_STATISTICS statistics;
    ID3D11GeometryShader *loop_gs, *vs_gs;
    ID3D11InputLayout *position_layout;
    ID3D11VertexShader *position_vs;
    ID3D11Buffer *vb, *ibs[2], *args, *large;
    ID3D11Query *query;
    UINT stride = 16, offset = 0;
    unsigned d, p, v, c, primitives, failed = 0;

    for (v = 0; v < CUT_BLOCKS_VERTICES; v++)
    {
        vertices[v][0] = (float)v;
        vertices[v][1] = 0.0f;
        vertices[v][2] = 0.5f;
        vertices[v][3] = 1.0f;
    }
    for (p = 0; p < CUT_BLOCKS_COUNT; p++)
    {
        indices16[CUT_BLOCKS_START + p] = is_cut_position(p) ? 0xffff : p;
        indices32[CUT_BLOCKS_START + p] = is_cut_position(p) ? 0xffffffff : p;
    }
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, triangle_gs_code, sizeof(triangle_gs_code),
                                                               decl, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM,
                                                               NULL, &loop_gs),
             "CreateGeometryShaderWithStreamOutput");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, position_vs_code, sizeof(position_vs_code),
                                                               decl, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM,
                                                               NULL, &vs_gs),
             "CreateGeometryShaderWithStreamOutput of a vertex shader");
    check_hr(ID3D11Device_CreateInputLayout(device, layout_desc, 1, position_vs_code, sizeof(position_vs_code),
                                            &position_layout),
             "CreateInputLayout");
    check_hr(ID3D11Device_CreateVertexShader(device, position_vs_code, sizeof(position_vs_code), NULL, &position_vs),
             "CreateVertexShader");
    vb = create_data_buffer(vertices, sizeof(vertices), D3D11_BIND_VERTEX_BUFFER, 0);
    ibs[0] = create_data_buffer(indices16, sizeof(indices16), D3D11_BIND_INDEX_BUFFER, 0);
    ibs[1] = create_data_buffer(indices32, sizeof(indices32), D3D11_BIND_INDEX_BUFFER, 0);
    args = create_data_buffer(arguments, sizeof(arguments), 0, D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS);
    query = create_query(D3D11_QUERY_SO_STATISTICS);
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, position_layout);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &stride, &offset);
    ID3D11DeviceContext_VSSetShader(context, position_vs, NULL, 0);
    for (d = 0; d < 2 * ARRAY_SIZE(names); d++)
    {
        unsigned f = d / ARRAY_SIZE(names), i = d % ARRAY_SIZE(names);
        unsigned per_primitive = i == 4 ? 2 : 3, instance, bad = 0;
        int ok;

        ID3D11DeviceContext_IASetIndexBuffer(context, ibs[f], f ? DXGI_FORMAT_R32_UINT : DXGI_FORMAT_R16_UINT, 0);
        ID3D11DeviceContext_IASetPrimitiveTopology(context, i == 4 ? D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP
                                                                   : D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        ID3D11DeviceContext_GSSetShader(context, i < 2 ? loop_gs : vs_gs, NULL, 0);
        /* A new target, so that a draw that writes nothing does not find the
           primitives of the draw before */
        check_hr(ID3D11Device_CreateBuffer(device, &target_desc, NULL, &large), "CreateBuffer");
        bind_target(large, 0);
        ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)query);
        if (i == 1 || i == 3)
            ID3D11DeviceContext_DrawIndexedInstancedIndirect(context, args, 0);
        else
            ID3D11DeviceContext_DrawIndexedInstanced(context, CUT_BLOCKS_COUNT, CUT_BLOCKS_INSTANCES, CUT_BLOCKS_START,
                                                     0, 0);
        ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)query);
        get_query_data(query, &statistics, sizeof(statistics));
        read_buffer_size(large, data, sizeof(data));
        bind_target(NULL, 0);
        ID3D11Buffer_Release(large);
        primitives = cut_blocks_primitives(per_primitive, want);
        for (instance = 0; instance < CUT_BLOCKS_INSTANCES; instance++)
        {
            for (v = 0; v < primitives * per_primitive; v++)
            {
                const float *vertex = (const float *)&data[((instance * primitives * per_primitive) + v) * 4];

                for (c = 0; c < 4; c++)
                    if (vertex[c] != vertices[want[v]][c])
                    {
                        if (!bad) say("%s, %s: instance %u, vertex %u: %g, want %g", names[i], formats[f], instance,
                                      v, vertex[0], vertices[want[v]][0]);
                        bad++;
                        break;
                    }
            }
        }
        ok = !bad && statistics.NumPrimitivesWritten == CUT_BLOCKS_INSTANCES * primitives
             && statistics.PrimitivesStorageNeeded == CUT_BLOCKS_INSTANCES * primitives;
        say("%s, %s: %u vertices wrong, primitives written %u, storage needed %u (want %u)", names[i], formats[f],
            bad, (unsigned)statistics.NumPrimitivesWritten, (unsigned)statistics.PrimitivesStorageNeeded,
            CUT_BLOCKS_INSTANCES * primitives);
        if (!ok) failed++;
    }
    ID3D11DeviceContext_ClearState(context);
    ID3D11Query_Release(query);
    ID3D11Buffer_Release(args);
    ID3D11Buffer_Release(ibs[1]);
    ID3D11Buffer_Release(ibs[0]);
    ID3D11Buffer_Release(vb);
    ID3D11VertexShader_Release(position_vs);
    ID3D11InputLayout_Release(position_layout);
    ID3D11GeometryShader_Release(vs_gs);
    ID3D11GeometryShader_Release(loop_gs);
    if (failed)
        fail(1, "%u of %u draws (want 0) wrote other primitives or counts", failed, 2 * (unsigned)ARRAY_SIZE(names));
}

/* A D3D11 geometry shader that passes its triangles and the layer of layer_vs
 * through (layer_pass_gs), created with rasterized stream 0: the strip of two
 * triangles over the render target is streamed out as a list and drawn into
 * slice 1 of a render target of two, slice 0 left alone, and DrawAuto from
 * the target draws the list again */
static void geometry_pass_through_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "SV_Position", 0, 0, 4, 0 } };
    static const unsigned list[] = { 0, 1, 2, 1, 3, 2 };
    static const float red[] = { 1.0f, 0.0f, 0.0f, 1.0f };
    static uint32_t code[CONTAINER_WORDS], vs_code_layer[CONTAINER_WORDS], data[TARGET_WORDS];
    ID3D11RenderTargetView *layered_rtv;
    ID3D11GeometryShader *pass_gs;
    ID3D11Texture2D *layered_rt;
    ID3D11VertexShader *layer_vs;
    ID3D11Buffer *auto_target;
    UINT stride = 16, offset = 0;
    uint32_t drawn, drawn_slice0, drawn_auto, size;
    int ok;

    size = build_layer_vs(vs_code_layer);
    check_hr(ID3D11Device_CreateVertexShader(device, vs_code_layer, size, NULL, &layer_vs), "CreateVertexShader");
    size = build_layer_pass_gs(code);
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, 1, &stride, 1, 0, NULL,
                                                               &pass_gs),
             "CreateGeometryShaderWithStreamOutput of a pass-through shader");
    create_layered_rt(&layered_rt, &layered_rtv);
    auto_target = create_buffer_bind(TARGET_SIZE, TARGET_FILL, D3D11_BIND_STREAM_OUTPUT | D3D11_BIND_VERTEX_BUFFER);
    bind_quad();
    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &layered_rtv, NULL);
    ID3D11DeviceContext_ClearRenderTargetView(context, layered_rtv, red);
    ID3D11DeviceContext_VSSetShader(context, layer_vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, pass_gs, NULL, 0);
    bind_target(auto_target, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    drawn = read_slice_pixel(layered_rt, 1, 10, 20);
    drawn_slice0 = read_slice_pixel(layered_rt, 0, 10, 20);
    read_buffer(auto_target, data);
    /* DrawAuto through the vertex shader of the quad into the render target
       of one slice */
    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &rtv, NULL);
    ID3D11DeviceContext_ClearRenderTargetView(context, rtv, red);
    ID3D11DeviceContext_VSSetShader(context, quad_vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, NULL, NULL, 0);
    ID3D11DeviceContext_SOSetTargets(context, 0, NULL, NULL);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &auto_target, &stride, &offset);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_DrawAuto(context);
    drawn_auto = read_pixel(10, 20);
    release_quad();
    ID3D11Buffer_Release(auto_target);
    ID3D11RenderTargetView_Release(layered_rtv);
    ID3D11Texture2D_Release(layered_rt);
    ID3D11GeometryShader_Release(pass_gs);
    ID3D11VertexShader_Release(layer_vs);
    ok = has_quad(data, 0, ARRAY_SIZE(list), list) && !count_changed(data, 24, TARGET_WORDS, TARGET_FILL);
    say("the two triangles %s, slice 1 %#x, slice 0 %#x, after DrawAuto %#x", ok ? "written" : "not written", drawn,
        drawn_slice0, drawn_auto);
    if (!ok || drawn != GREEN || drawn_slice0 != RED || drawn_auto != GREEN)
        fail(1, "want the two triangles written, slice 1 %#x, slice 0 %#x, after DrawAuto %#x", GREEN, RED, GREEN);
}

#define PARTICLE_FRAMES 5

/* The particles of particle_gs_shdr after <frames> frames from 8 particles
 * (0, 10 * i, 0, 1), as the GPU appends them, into <want>. Returns their
 * number. */
static unsigned particles(float (*want)[4], unsigned frames)
{
    static float next[LARGE_WORDS / 4][4];
    unsigned n = 8, frame, i, m;

    for (i = 0; i < n; i++)
    {
        want[i][0] = want[i][2] = 0.0f;
        want[i][1] = 10.0f * (float)i;
        want[i][3] = 1.0f;
    }
    for (frame = 0; frame < frames; frame++)
    {
        for (i = 0, m = 0; i < n; i++)
        {
            next[m][0] = want[i][0] + 1.0f;
            next[m][1] = want[i][1];
            next[m][2] = want[i][2];
            next[m++][3] = want[i][3];
            if (want[i][0] < 2.0f)
            {
                next[m][0] = want[i][0] + 1.0f;
                next[m][1] = want[i][1] + 1.0f;
                next[m][2] = want[i][2];
                next[m++][3] = want[i][3];
            }
        }
        memcpy(want, next, m * sizeof(*next));
        n = m;
    }
    return n;
}

/* A geometry shader particle system: two targets take turns, each frame a
 * DrawAuto through the geometry shader of the one the frame before filled,
 * whose filled size only the GPU knows. The last one is then drawn with
 * DrawAuto through a vertex shader that streams out into a third target. */
static void geometry_particles_case(void)
{
    static const D3D11_INPUT_ELEMENT_DESC layout_desc[] = {
        { "POSITION", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0 },
    };
    static const D3D11_SO_DECLARATION_ENTRY decl[] = { { 0, "SV_Position", 0, 0, 4, 0 } };
    static uint32_t code[CONTAINER_WORDS], data[LARGE_WORDS], copy[LARGE_WORDS];
    static float want[LARGE_WORDS / 4][4], initial[8][4];
    ID3D11GeometryShader *particle_gs, *copy_gs;
    ID3D11Buffer *initial_vb, *ping[2], *copied;
    ID3D11InputLayout *position_layout;
    ID3D11VertexShader *position_vs;
    UINT stride = 16, offset = 0;
    unsigned frame, n, i;
    uint32_t size;
    int ok, ok_copy;

    n = particles(initial, 0);
    initial_vb = create_data_buffer(initial, sizeof(initial), D3D11_BIND_VERTEX_BUFFER, 0);
    for (i = 0; i < 2; i++)
        ping[i] = create_buffer_bind(LARGE_SIZE, TARGET_FILL, D3D11_BIND_STREAM_OUTPUT | D3D11_BIND_VERTEX_BUFFER);
    copied = create_buffer(LARGE_SIZE, TARGET_FILL);
    size = build_position_gs(code, particle_gs_shdr, ARRAY_SIZE(particle_gs_shdr));
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, code, size, decl, 1, &stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &particle_gs),
             "CreateGeometryShaderWithStreamOutput");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, position_vs_code, sizeof(position_vs_code),
                                                               decl, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM,
                                                               NULL, &copy_gs),
             "CreateGeometryShaderWithStreamOutput");
    check_hr(ID3D11Device_CreateInputLayout(device, layout_desc, 1, position_vs_code, sizeof(position_vs_code),
                                            &position_layout),
             "CreateInputLayout");
    check_hr(ID3D11Device_CreateVertexShader(device, position_vs_code, sizeof(position_vs_code), NULL, &position_vs),
             "CreateVertexShader");
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetInputLayout(context, position_layout);
    ID3D11DeviceContext_VSSetShader(context, position_vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, particle_gs, NULL, 0);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &initial_vb, &stride, &offset);
    bind_target(ping[0], 0);
    ID3D11DeviceContext_Draw(context, n, 0);
    for (frame = 1; frame < PARTICLE_FRAMES; frame++)
    {
        bind_target(ping[frame & 1], 0);
        ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &ping[(frame - 1) & 1], &stride, &offset);
        ID3D11DeviceContext_DrawAuto(context);
    }
    ID3D11DeviceContext_GSSetShader(context, copy_gs, NULL, 0);
    bind_target(copied, 0);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &ping[(PARTICLE_FRAMES - 1) & 1], &stride, &offset);
    ID3D11DeviceContext_DrawAuto(context);
    read_buffer_size(ping[(PARTICLE_FRAMES - 1) & 1], data, LARGE_SIZE);
    read_buffer_size(copied, copy, LARGE_SIZE);
    ID3D11DeviceContext_ClearState(context);
    ID3D11VertexShader_Release(position_vs);
    ID3D11InputLayout_Release(position_layout);
    ID3D11GeometryShader_Release(copy_gs);
    ID3D11GeometryShader_Release(particle_gs);
    ID3D11Buffer_Release(copied);
    for (i = 0; i < 2; i++) ID3D11Buffer_Release(ping[i]);
    ID3D11Buffer_Release(initial_vb);

    n = particles(want, PARTICLE_FRAMES);
    ok = has_points(data, 0, (const float (*)[4])want, n) && !count_changed(data, n * 4, LARGE_WORDS, TARGET_FILL);
    ok_copy = has_points(copy, 0, (const float (*)[4])want, n) && !count_changed(copy, n * 4, LARGE_WORDS, TARGET_FILL);
    if (!ok || !ok_copy)
        fail(1, "the %u particles after %u frames %s; their copy through a vertex shader %s", n, PARTICLE_FRAMES,
             ok ? "correct" : "wrong", ok_copy ? "correct" : "wrong");
}

/* The four streams over 1024, 1025 and 2048 points: one block of 1024 mesh
 * threadgroups, two whose last holds the 1025th point alone, and two full
 * ones. Each stream takes its own number of points, all of which fit. */
static void geometry_streams_blocks_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = {
        { 0, "custom", 0, 0, 4, 0 },
        { 1, "custom", 0, 0, 4, 1 },
        { 2, "custom", 0, 0, 4, 2 },
        { 3, "custom", 0, 0, 4, 3 },
    };
    static const D3D11_QUERY types[] = {
        D3D11_QUERY_SO_STATISTICS_STREAM0, D3D11_QUERY_SO_STATISTICS_STREAM1,
        D3D11_QUERY_SO_STATISTICS_STREAM2, D3D11_QUERY_SO_STATISTICS_STREAM3,
    };
    static const UINT sizes[] = { 1024, 1025, 2048 };
    static uint32_t data[2048 * 4];
    D3D11_BUFFER_DESC desc = { sizeof(data), D3D11_USAGE_DEFAULT, D3D11_BIND_STREAM_OUTPUT, 0, 0, 0 };
    D3D11_QUERY_DATA_SO_STATISTICS statistics;
    UINT offsets[4] = { 0, 0, 0, 0 };
    ID3D11GeometryShader *streams_gs;
    ID3D11Buffer *targets[4], *cb;
    ID3D11VertexShader *id_vs;
    ID3D11Query *queries[4];
    unsigned i, s, v, c, correct = 0;

    check_hr(ID3D11Device_CreateVertexShader(device, id_vs_code, sizeof(id_vs_code), NULL, &id_vs),
             "CreateVertexShader");
    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, streams_gs_code, sizeof(streams_gs_code), decl,
                                                               4, NULL, 0, D3D11_SO_NO_RASTERIZED_STREAM, NULL,
                                                               &streams_gs),
             "CreateGeometryShaderWithStreamOutput");
    for (s = 0; s < 4; s++) queries[s] = create_query(types[s]);
    for (i = 0; i < ARRAY_SIZE(sizes); i++)
    {
        uint32_t counts[4] = { sizes[i], sizes[i] - 1, sizes[i] / 2, 1 };

        cb = create_data_buffer(counts, sizeof(counts), D3D11_BIND_CONSTANT_BUFFER, 0);
        for (s = 0; s < 4; s++)
            check_hr(ID3D11Device_CreateBuffer(device, &desc, NULL, &targets[s]), "CreateBuffer");
        bind_pipeline(vb0);
        ID3D11DeviceContext_IASetInputLayout(context, NULL);
        ID3D11DeviceContext_VSSetShader(context, id_vs, NULL, 0);
        ID3D11DeviceContext_GSSetShader(context, streams_gs, NULL, 0);
        ID3D11DeviceContext_GSSetConstantBuffers(context, 0, 1, &cb);
        ID3D11DeviceContext_SOSetTargets(context, 4, targets, offsets);
        for (s = 0; s < 4; s++) ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)queries[s]);
        ID3D11DeviceContext_Draw(context, sizes[i], 0);
        for (s = 0; s < 4; s++) ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)queries[s]);
        for (s = 0; s < 4; s++)
        {
            int same = 1;

            get_query_data(queries[s], &statistics, sizeof(statistics));
            read_buffer_size(targets[s], data, sizeof(data));
            for (v = 0; v < counts[s]; v++)
                for (c = 0; c < 4; c++) same &= ((const float *)data)[v * 4 + c] == (float)v;
            if (same && statistics.NumPrimitivesWritten == counts[s] && statistics.PrimitivesStorageNeeded == counts[s])
                correct++;
            else
                say("%u points, stream %u: points %s, written %u, needed %u (want %u)", sizes[i], s,
                    same ? "in order" : "not in order", (unsigned)statistics.NumPrimitivesWritten,
                    (unsigned)statistics.PrimitivesStorageNeeded, counts[s]);
        }
        ID3D11DeviceContext_ClearState(context);
        for (s = 0; s < 4; s++) ID3D11Buffer_Release(targets[s]);
        ID3D11Buffer_Release(cb);
    }
    for (s = 0; s < 4; s++) ID3D11Query_Release(queries[s]);
    ID3D11GeometryShader_Release(streams_gs);
    ID3D11VertexShader_Release(id_vs);
    if (correct != 4 * ARRAY_SIZE(sizes)) fail(1, "streams with their points and counts %u/12", correct);
}

static void set_vertices(ID3D11Buffer *vb)
{
    UINT stride = VERTEX_STRIDE, offset = 0;

    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &stride, &offset);
}

static void append_draws_case(void)
{
    static const unsigned first[] = { 0, 1, 2, 3 }, second[] = { 0, 1, 2, 3, 4, 5, 6, 7 }, last[] = { 5 };
    static uint32_t data[TARGET_WORDS];
    ID3D11Buffer *null_buffer = NULL;
    UINT zero = 0;
    int ok;

    bind_pipeline(vb0);
    bind_target(target, 0);
    ID3D11DeviceContext_Draw(context, 4, 0);
    set_vertices(vb1);
    ID3D11DeviceContext_Draw(context, 8, 0);
    bind_target(target, 0xffffffff);
    ID3D11DeviceContext_Draw(context, 2, 0);
    ID3D11DeviceContext_SOSetTargets(context, 1, &null_buffer, &zero);
    bind_target(target, 0xffffffff);
    set_vertices(vb0);
    ID3D11DeviceContext_Draw(context, 1, 5);
    read_buffer(target, data);
    ok = has_list(data, 4, 0, 4, 0, first) && has_list(data, 4, 4, 8, 1, second) && has_list(data, 4, 12, 2, 1, second)
         && has_list(data, 4, 14, 1, 0, last);
    if (!ok || count_changed(data, 60, TARGET_WORDS, TARGET_FILL))
        fail(1, "the 15 vertices of the four draws one after another %s, words after them changed %u (want 0)",
             ok ? "written" : "not written", count_changed(data, 60, TARGET_WORDS, TARGET_FILL));
}

/* Both draws of a round are in one render pass, so the GPU runs them in any
 * order unless DXMT orders them. */
static void rewrite_case(void)
{
    static const unsigned kept[] = { 4, 5, 6, 7 };
    static uint32_t data[TARGET_WORDS];
    unsigned round;

    bind_pipeline(vb0);
    for (round = 0; round < 16; round++)
    {
        set_vertices(vb0);
        bind_target(target, 0);
        ID3D11DeviceContext_Draw(context, 8, 0);
        set_vertices(vb1);
        bind_target(target, 0);
        ID3D11DeviceContext_Draw(context, 4, 0);
        read_buffer(target, data);
        if (!has_vertices(data, 0, 4, 1) || !has_list(data, 4, 4, 4, 0, kept))
            fail(1, "round %u: the second draw's 4 vertices %s, the first draw's last 4 %s", round,
                 has_vertices(data, 0, 4, 1) ? "written" : "not written",
                 has_list(data, 4, 4, 4, 0, kept) ? "kept" : "not kept");
    }
}

static void overflow_case(void)
{
    static uint32_t data[TARGET_WORDS];
    D3D11_QUERY_DATA_SO_STATISTICS statistics;
    ID3D11Query *statistics_query, *overflow_query;
    BOOL overflow;

    statistics_query = create_query(D3D11_QUERY_SO_STATISTICS);
    overflow_query = create_query(D3D11_QUERY_SO_OVERFLOW_PREDICATE);
    bind_pipeline(vb0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    bind_target(target, 0);

    /* 4 triangles, then 2 and 2 vertices of a third, with room for one */
    ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)statistics_query);
    ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)overflow_query);
    ID3D11DeviceContext_Draw(context, 12, 0);
    ID3D11DeviceContext_Draw(context, 8, 12);
    ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)overflow_query);
    ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)statistics_query);
    get_query_data(statistics_query, &statistics, sizeof(statistics));
    get_query_data(overflow_query, &overflow, sizeof(overflow));
    read_buffer(target, data);
    if (statistics.NumPrimitivesWritten != 5 || statistics.PrimitivesStorageNeeded != 6 || !overflow
        || !has_vertices(data, 0, 15, 0) || count_changed(data, 60, TARGET_WORDS, TARGET_FILL))
        fail(1, "primitives written %u (want 5), storage needed %u (want 6), overflow %d (want 1), the 5 triangles "
             "%s, words after them changed %u (want 0)", (unsigned)statistics.NumPrimitivesWritten,
             (unsigned)statistics.PrimitivesStorageNeeded, overflow,
             has_vertices(data, 0, 15, 0) ? "written" : "not written",
             count_changed(data, 60, TARGET_WORDS, TARGET_FILL));

    /* A triangle into the full target, then one into the target bound again */
    ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)statistics_query);
    ID3D11DeviceContext_Draw(context, 3, 0);
    ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)statistics_query);
    get_query_data(statistics_query, &statistics, sizeof(statistics));
    if (statistics.NumPrimitivesWritten != 0 || statistics.PrimitivesStorageNeeded != 1)
        fail(1, "full target: primitives written %u (want 0), storage needed %u (want 1)",
             (unsigned)statistics.NumPrimitivesWritten, (unsigned)statistics.PrimitivesStorageNeeded);
    bind_target(target, 0);
    ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)overflow_query);
    ID3D11DeviceContext_Draw(context, 3, 0);
    ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)overflow_query);
    get_query_data(overflow_query, &overflow, sizeof(overflow));
    if (overflow) fail(1, "overflow %d for a triangle that fits (want 0)", overflow);

    ID3D11Query_Release(overflow_query);
    ID3D11Query_Release(statistics_query);
}

static void topology_case(void)
{
    static const uint16_t indices16[] = { 3, 2, 1, 0, 1, 3 };
    static const uint32_t indices32[] = { 63, 7, 5, 6 };
    static const unsigned indexed[] = { 4, 3, 2, 3, 5, 7, 5, 6, 8, 9, 9, 10, 12, 13 };
    static const unsigned strips[] = { 20, 21, 22, 21, 23, 22, 22, 23, 24, 30, 31, 30, 31, 30, 31 };
    static uint32_t data[TARGET_WORDS], data_strips[TARGET_WORDS];
    ID3D11Buffer *ib16, *ib32, *target_strips;
    int ok, ok_strips;

    ib16 = create_data_buffer(indices16, sizeof(indices16), D3D11_BIND_INDEX_BUFFER, 0);
    ib32 = create_data_buffer(indices32, sizeof(indices32), D3D11_BIND_INDEX_BUFFER, 0);
    target_strips = create_buffer(TARGET_SIZE, TARGET_FILL);

    bind_pipeline(vb0);
    bind_target(target, 0);
    ID3D11DeviceContext_IASetIndexBuffer(context, ib16, DXGI_FORMAT_R16_UINT, 0);
    ID3D11DeviceContext_DrawIndexed(context, 5, 1, 2);
    ID3D11DeviceContext_IASetIndexBuffer(context, ib32, DXGI_FORMAT_R32_UINT, 4);
    ID3D11DeviceContext_DrawIndexed(context, 3, 0, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP);
    ID3D11DeviceContext_Draw(context, 3, 8);
    /* One line; the third vertex completes none */
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_LINELIST);
    ID3D11DeviceContext_Draw(context, 3, 12);

    /* The first instance is not instance 0: it still comes first */
    bind_target(target_strips, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
    ID3D11DeviceContext_Draw(context, 5, 20);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
    ID3D11DeviceContext_DrawInstanced(context, 2, 3, 30, 1);

    read_buffer(target, data);
    read_buffer(target_strips, data_strips);
    ID3D11DeviceContext_ClearState(context);
    ID3D11Buffer_Release(target_strips);
    ID3D11Buffer_Release(ib32);
    ID3D11Buffer_Release(ib16);
    ok = has_list(data, 4, 0, ARRAY_SIZE(indexed), 0, indexed);
    ok_strips = has_list(data_strips, 4, 0, ARRAY_SIZE(strips), 0, strips);
    if (!ok || !ok_strips || count_changed(data, 56, TARGET_WORDS, TARGET_FILL)
        || count_changed(data_strips, 60, TARGET_WORDS, TARGET_FILL))
        fail(1, "the indexed draws, the line strip and the line list %s, the triangle strip and the instances %s, "
             "words after them changed %u and %u (want 0)", ok ? "in order" : "not in order",
             ok_strips ? "in order" : "not in order", count_changed(data, 56, TARGET_WORDS, TARGET_FILL),
             count_changed(data_strips, 60, TARGET_WORDS, TARGET_FILL));
}

/* A topology with adjacency streams out its primitives without the adjacent
 * vertices: a line of 4 vertices, two lines of a strip of 5, and a triangle
 * of 6 vertices as a list and as a strip. */
static void adjacency_case(void)
{
    static const unsigned streamed[] = { 1, 2, 5, 6, 6, 7, 10, 12, 14, 20, 22, 24 };
    static uint32_t data[TARGET_WORDS];
    int ok;

    bind_pipeline(vb0);
    bind_target(target, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_LINELIST_ADJ);
    ID3D11DeviceContext_Draw(context, 4, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP_ADJ);
    ID3D11DeviceContext_Draw(context, 5, 4);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST_ADJ);
    ID3D11DeviceContext_Draw(context, 6, 10);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP_ADJ);
    ID3D11DeviceContext_Draw(context, 6, 20);

    read_buffer(target, data);
    ID3D11DeviceContext_ClearState(context);
    ok = has_list(data, 4, 0, ARRAY_SIZE(streamed), 0, streamed);
    if (!ok || count_changed(data, 48, TARGET_WORDS, TARGET_FILL))
        fail(1, "the line, the two lines of the strip and the two triangles %s without their adjacent vertices, "
             "words after them changed %u (want 0)", ok ? "written" : "not written",
             count_changed(data, 48, TARGET_WORDS, TARGET_FILL));
}

/* {POSITION .xy} goes to slot 0 and {POSITION .zw} to slot 1, 8 bytes each.
 * Slot 1 is bound 24 bytes before its end, so both take 3 of the 8 vertices. */
static void slots_case(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = {
        { 0, "POSITION", 0, 0, 2, 0 },
        { 0, "POSITION", 0, 2, 2, 1 },
    };
    static const UINT strides[] = { 8, 8 };
    static uint32_t data[TARGET_WORDS], data_b[TARGET_WORDS];
    UINT offsets[] = { 0, TARGET_SIZE - 24 };
    ID3D11GeometryShader *slots_gs;
    ID3D11Buffer *targets[2];
    unsigned correct = 0, v;

    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, vs_code, sizeof(vs_code), decl, 2, strides, 2,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &slots_gs),
             "CreateGeometryShaderWithStreamOutput");
    targets[0] = target;
    targets[1] = create_buffer(TARGET_SIZE, TARGET_FILL);
    bind_pipeline(vb0);
    ID3D11DeviceContext_GSSetShader(context, slots_gs, NULL, 0);
    ID3D11DeviceContext_SOSetTargets(context, 2, targets, offsets);
    ID3D11DeviceContext_Draw(context, 8, 0);
    /* Slot 1 without a target counts as full */
    offsets[0] = 64;
    ID3D11DeviceContext_SOSetTargets(context, 1, targets, offsets);
    ID3D11DeviceContext_Draw(context, 4, 0);
    read_buffer(targets[0], data);
    read_buffer(targets[1], data_b);
    ID3D11DeviceContext_ClearState(context);
    ID3D11Buffer_Release(targets[1]);
    ID3D11GeometryShader_Release(slots_gs);
    for (v = 0; v < 3; v++)
    {
        const float *xy = (const float *)&data[v * 2], *zw = (const float *)&data_b[TARGET_WORDS - 6 + v * 2];
        correct += xy[0] == position(0, v, 0) && xy[1] == position(0, v, 1) && zw[0] == position(0, v, 2)
                   && zw[1] == position(0, v, 3);
    }
    if (correct != 3 || count_changed(data, 0, TARGET_WORDS, TARGET_FILL) != 6
        || count_changed(data_b, 0, TARGET_WORDS, TARGET_FILL) != 6)
        fail(1, "vertices with .xy in slot 0 and .zw in slot 1 %u/3, words changed %u and %u (want 6 and 6)", correct,
             count_changed(data, 0, TARGET_WORDS, TARGET_FILL), count_changed(data_b, 0, TARGET_WORDS, TARGET_FILL));
}

/* A geometry shader object that streams out whole vertices of vs_code, so a
 * target it fills can be the vertex buffer of the next draw */
static ID3D11GeometryShader *create_vertex_gs(void)
{
    static const D3D11_SO_DECLARATION_ENTRY decl[] = {
        { 0, "POSITION", 0, 0, 4, 0 },
        { 0, "COLOR", 0, 0, 4, 0 },
        { 0, "COLOR", 1, 0, 4, 0 },
    };
    ID3D11GeometryShader *vertex_gs;
    UINT stride = VERTEX_STRIDE;

    check_hr(ID3D11Device_CreateGeometryShaderWithStreamOutput(device, vs_code, sizeof(vs_code), decl, 3, &stride, 1,
                                                               D3D11_SO_NO_RASTERIZED_STREAM, NULL, &vertex_gs),
             "CreateGeometryShaderWithStreamOutput");
    return vertex_gs;
}

static void draw_auto_case(void)
{
    static const unsigned streamed[] = { 1, 2, 3, 10, 11 };
    static uint32_t data[TARGET_WORDS], data_vertices[TARGET_WORDS];
    UINT stride = VERTEX_STRIDE, offset = VERTEX_STRIDE;
    ID3D11GeometryShader *vertex_gs = create_vertex_gs();
    ID3D11Buffer *vertices;
    int ok, ok_auto;

    vertices = create_buffer_bind(TARGET_SIZE, TARGET_FILL, D3D11_BIND_STREAM_OUTPUT | D3D11_BIND_VERTEX_BUFFER);
    bind_pipeline(vb0);
    ID3D11DeviceContext_GSSetShader(context, vertex_gs, NULL, 0);
    bind_target(vertices, 0);
    ID3D11DeviceContext_Draw(context, 3, 1);
    ID3D11DeviceContext_Draw(context, 2, 10);

    /* From the second of the 5 vertices on, into slots 1 to 4 */
    ID3D11DeviceContext_GSSetShader(context, gs, NULL, 0);
    bind_target(target, 16);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vertices, &stride, &offset);
    ID3D11DeviceContext_DrawAuto(context);

    read_buffer(vertices, data_vertices);
    read_buffer(target, data);
    ID3D11DeviceContext_ClearState(context);
    ID3D11Buffer_Release(vertices);
    ID3D11GeometryShader_Release(vertex_gs);
    ok = has_list(data_vertices, VERTEX_STRIDE / 4, 0, 5, 0, streamed);
    ok_auto = has_list(data, 4, 1, 4, 0, streamed + 1);
    if (!ok || !ok_auto || count_changed(data, 0, 4, TARGET_FILL) || count_changed(data, 20, TARGET_WORDS, TARGET_FILL))
        fail(1, "the 5 vertices of the two draws %s, the 4 of them DrawAuto draws %s, words around them changed %u "
             "(want 0)", ok ? "written" : "not written", ok_auto ? "written" : "not written",
             count_changed(data, 0, 4, TARGET_FILL) + count_changed(data, 20, TARGET_WORDS, TARGET_FILL));
}

/* Only the GPU knows what an indirect draw writes, so it has to count for the
 * draws after it and for the queries. */
static void indirect_case(void)
{
    static const uint32_t arguments[] = {
        4, 1, 2, 0,    /* DrawInstancedIndirect: vertices 2 to 5 */
        3, 1, 1, 2, 0, /* DrawIndexedInstancedIndirect: indices 1 to 3 on base vertex 2 */
    };
    static const uint16_t indices[] = { 3, 2, 1, 0 };
    static const unsigned streamed[] = { 2, 3, 4, 5, 4 }, appended[] = { 0, 1 };
    static uint32_t data[TARGET_WORDS], data_vertices[TARGET_WORDS];
    UINT stride = VERTEX_STRIDE, offset = VERTEX_STRIDE;
    ID3D11GeometryShader *vertex_gs = create_vertex_gs();
    D3D11_QUERY_DATA_SO_STATISTICS statistics;
    ID3D11Buffer *vertices, *args, *ib;
    ID3D11Query *statistics_query;
    int ok, ok_auto;

    vertices = create_buffer_bind(TARGET_SIZE, TARGET_FILL, D3D11_BIND_STREAM_OUTPUT | D3D11_BIND_VERTEX_BUFFER);
    args = create_data_buffer(arguments, sizeof(arguments), 0, D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS);
    ib = create_data_buffer(indices, sizeof(indices), D3D11_BIND_INDEX_BUFFER, 0);
    statistics_query = create_query(D3D11_QUERY_SO_STATISTICS);

    /* 4 vertices, then 3 with room for one */
    bind_pipeline(vb0);
    ID3D11DeviceContext_GSSetShader(context, vertex_gs, NULL, 0);
    ID3D11DeviceContext_IASetIndexBuffer(context, ib, DXGI_FORMAT_R16_UINT, 0);
    bind_target(vertices, 0);
    ID3D11DeviceContext_Begin(context, (ID3D11Asynchronous *)statistics_query);
    ID3D11DeviceContext_DrawInstancedIndirect(context, args, 0);
    ID3D11DeviceContext_DrawIndexedInstancedIndirect(context, args, 16);
    ID3D11DeviceContext_End(context, (ID3D11Asynchronous *)statistics_query);

    /* DrawAuto from the second vertex on, then two vertices after them */
    ID3D11DeviceContext_GSSetShader(context, gs, NULL, 0);
    bind_target(target, 0);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vertices, &stride, &offset);
    ID3D11DeviceContext_DrawAuto(context);
    set_vertices(vb1);
    ID3D11DeviceContext_Draw(context, 2, 0);

    get_query_data(statistics_query, &statistics, sizeof(statistics));
    read_buffer(vertices, data_vertices);
    read_buffer(target, data);
    ID3D11DeviceContext_ClearState(context);
    ID3D11Query_Release(statistics_query);
    ID3D11Buffer_Release(ib);
    ID3D11Buffer_Release(args);
    ID3D11Buffer_Release(vertices);
    ID3D11GeometryShader_Release(vertex_gs);
    ok = has_list(data_vertices, VERTEX_STRIDE / 4, 0, 5, 0, streamed);
    ok_auto = has_list(data, 4, 0, 4, 0, streamed + 1) && has_list(data, 4, 4, 2, 1, appended);
    if (statistics.NumPrimitivesWritten != 5 || statistics.PrimitivesStorageNeeded != 7 || !ok || !ok_auto
        || count_changed(data, 24, TARGET_WORDS, TARGET_FILL))
        fail(1, "primitives written %u (want 5), storage needed %u (want 7), the 5 vertices of the indirect draws "
             "%s, the 4 DrawAuto draws and the 2 after them %s, words after them changed %u (want 0)",
             (unsigned)statistics.NumPrimitivesWritten, (unsigned)statistics.PrimitivesStorageNeeded,
             ok ? "written" : "not written", ok_auto ? "written" : "not written",
             count_changed(data, 24, TARGET_WORDS, TARGET_FILL));
}

/* A command list binds the target to append, and draws once with arguments
 * only the GPU has and once without, inside a query. It runs twice: the
 * second run continues where the first ended, has room for one vertex, and
 * the query has the counts of the second run alone. */
static void deferred_case(void)
{
    static const uint32_t arguments[] = { 3, 1, 2, 0 }; /* vertices 2 to 4 */
    static const unsigned streamed[] = { 2, 3, 4, 10, 2 };
    static uint32_t data[TARGET_WORDS];
    D3D11_VIEWPORT viewport = { 0.0f, 0.0f, 64.0f, 64.0f, 0.0f, 1.0f };
    UINT stride = VERTEX_STRIDE, offset = 0, append = 0xffffffff;
    ID3D11GeometryShader *vertex_gs = create_vertex_gs();
    D3D11_QUERY_DATA_SO_STATISTICS first, second;
    ID3D11DeviceContext *deferred;
    ID3D11Query *statistics_query;
    ID3D11Buffer *vertices, *args;
    ID3D11CommandList *list;

    vertices = create_buffer(TARGET_SIZE, TARGET_FILL);
    args = create_data_buffer(arguments, sizeof(arguments), 0, D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS);
    statistics_query = create_query(D3D11_QUERY_SO_STATISTICS);
    check_hr(ID3D11Device_CreateDeferredContext(device, 0, &deferred), "CreateDeferredContext");

    ID3D11DeviceContext_OMSetRenderTargets(deferred, 1, &rtv, NULL);
    ID3D11DeviceContext_RSSetViewports(deferred, 1, &viewport);
    ID3D11DeviceContext_IASetInputLayout(deferred, layout);
    ID3D11DeviceContext_IASetVertexBuffers(deferred, 0, 1, &vb0, &stride, &offset);
    ID3D11DeviceContext_IASetPrimitiveTopology(deferred, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
    ID3D11DeviceContext_VSSetShader(deferred, vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(deferred, vertex_gs, NULL, 0);
    ID3D11DeviceContext_SOSetTargets(deferred, 1, &vertices, &append);
    ID3D11DeviceContext_Begin(deferred, (ID3D11Asynchronous *)statistics_query);
    ID3D11DeviceContext_DrawInstancedIndirect(deferred, args, 0);
    ID3D11DeviceContext_Draw(deferred, 1, 10);
    ID3D11DeviceContext_End(deferred, (ID3D11Asynchronous *)statistics_query);
    check_hr(ID3D11DeviceContext_FinishCommandList(deferred, FALSE, &list), "FinishCommandList");

    ID3D11DeviceContext_ExecuteCommandList(context, list, FALSE);
    get_query_data(statistics_query, &first, sizeof(first));
    ID3D11DeviceContext_ExecuteCommandList(context, list, FALSE);
    get_query_data(statistics_query, &second, sizeof(second));
    read_buffer(vertices, data);

    ID3D11CommandList_Release(list);
    ID3D11DeviceContext_Release(deferred);
    ID3D11Query_Release(statistics_query);
    ID3D11Buffer_Release(args);
    ID3D11Buffer_Release(vertices);
    ID3D11GeometryShader_Release(vertex_gs);
    if (first.NumPrimitivesWritten != 4 || first.PrimitivesStorageNeeded != 4 || second.NumPrimitivesWritten != 1
        || second.PrimitivesStorageNeeded != 4 || !has_list(data, VERTEX_STRIDE / 4, 0, 5, 0, streamed))
        fail(1, "first run: primitives written %u (want 4), storage needed %u (want 4); second run: written %u "
             "(want 1), needed %u (want 4); the 4 vertices of the first run and the one of the second %s",
             (unsigned)first.NumPrimitivesWritten, (unsigned)first.PrimitivesStorageNeeded,
             (unsigned)second.NumPrimitivesWritten, (unsigned)second.PrimitivesStorageNeeded,
             has_list(data, VERTEX_STRIDE / 4, 0, 5, 0, streamed) ? "written" : "not written");
}

static const struct
{
    const char *name;
    void (*run)(void);
    /* the feature level of the case's device, 11_0 when 0 */
    D3D_FEATURE_LEVEL level;
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
    { "rasterized", rasterized_case },
    { "geometry-cut", geometry_cut_case },
    { "geometry-cut-blocks", geometry_cut_blocks_case },
    { "geometry-pass-through", geometry_pass_through_case },
    { "geometry-rasterized", geometry_rasterized_case },
    { "geometry-streams-rasterized", geometry_streams_rasterized_case },
    { "geometry-uav", geometry_uav_case, D3D_FEATURE_LEVEL_11_1 },
    { "geometry-vs-uav", geometry_vs_uav_case, D3D_FEATURE_LEVEL_11_1 },
    { "geometry-shapes", geometry_shapes_case },
    { "geometry-strips", geometry_strips_case },
    { "geometry", geometry_case },
    { "geometry-order", geometry_order_case },
    { "geometry-large", geometry_large_case },
    { "geometry-indirect", geometry_indirect_case },
    { "geometry-particles", geometry_particles_case },
    { "geometry-streams", geometry_streams_case },
    { "geometry-streams-blocks", geometry_streams_blocks_case },
    { "append-draws", append_draws_case },
    { "rewrite", rewrite_case },
    { "overflow", overflow_case },
    { "topology", topology_case },
    { "slots", slots_case },
    { "draw-auto", draw_auto_case },
    { "indirect", indirect_case },
    { "deferred", deferred_case },
    { "adjacency", adjacency_case },
};

static void run_case(unsigned i)
{
    current = cases[i].name;
    say("case %s", current);
    setup(cases[i].level);
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
