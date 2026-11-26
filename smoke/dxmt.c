/* The DXMT program bundle-wine.sh builds for x86_64 and i686 and runs under
 * the bundled wine, once per case, with the case name as its argument. It
 * also builds for aarch64, for runs by hand:
 *
 * - bytecode: DXMT's D3D11 device reads shader bytecode within the length
 *   the application passes, not the container size the bytecode's own header
 *   states. CreateInputLayout and CreateGeometryShaderWithStreamOutput get a
 *   valid vertex shader with its full length, and the same bytecode with a
 *   length 16 bytes shorter than its header states. The first must succeed
 *   and the second fail. The buffer holds the whole shader either way, so the
 *   short length reads nothing the program did not initialise.
 * - swap-state: SwapDeviceContextState with the context state that is
 *   already installed, after the program released its own reference to it,
 *   keeps the state alive. The installed state is then the only holder.
 * - deferred: a deferred context the program releases after the device
 *   keeps the device alive and is destroyed before it.
 * - depth-bias: draws on a D24_UNORM_S8_UINT target, which DXMT stores in a
 *   float format, get the constant depth bias Direct3D gives a 24-bit
 *   buffer, DepthBias / (2^24 - 1), at a depth of 0.25, where Metal's own
 *   bias for a float buffer is half of it. Each check clears depth to 1 and
 *   the color to 0, draws a quad and reads back one pixel's depth and
 *   color: with a pixel shader at a DepthBias of +1000 and -1000, both in
 *   one render pass (the second draw switches DXMT's pipeline and bias, and
 *   under LESS its depth is the one left), with DepthBias 1000 and
 *   SlopeScaledDepthBias -2 on a sloped quad, the sloped quad without a
 *   pixel shader, and through a geometry shader (DXMT's mesh pipeline). The
 *   depth must be within 1 unit of 24 bits of the expected one, 3 with a
 *   slope term, the color the pixel shader's red, and without a pixel
 *   shader the cleared 0.
 * - geometry-index, tessellation-index: DXMT's geometry and tessellation
 *   pipelines (its mesh pipelines) load the indices themselves, with the
 *   index format they were built for, and read their draw arguments in the
 *   layout of an indexed or a non-indexed draw. Four units at different
 *   places, each a point the geometry shader makes a quad of, or a patch of
 *   four control points the hull and domain shaders make a quad of, are
 *   drawn two at a time in one render pass with nothing between the draws
 *   but IASetIndexBuffer: non-indexed then indexed, 16-bit then 32-bit
 *   indices, and the reverse of each. Each pass then reads back the centre
 *   of every unit, and exactly the two units drawn must be the pixel
 *   shader's red. The index values are chosen so that a draw on the
 *   pipeline of the draw before it lands on another unit.
 * - release-after-draw: a device released right after a draw is destroyed
 *   without a crash or a hang. With a 64x64 render target, an input layout,
 *   a vertex shader, a vertex buffer and a stream output buffer bound, and
 *   one of two geometry shaders created with stream output and no
 *   rasterized stream, the case unbinds the geometry shader, draws five
 *   points, flushes, clears the state and releases every object, the
 *   context and then the device. DXMT compiles the draw's shader variant
 *   and pipeline on worker threads and encodes the flushed draw on a thread
 *   of its command queue, which waits for that pipeline, so the device's
 *   release comes while both are still at work. The device must then be
 *   destroyed, and the process must get past its release.
 * - pass-through: DXMT takes a geometry shader that passes its triangles
 *   through as a pass-through, and no shader that differs from one. A quad
 *   patch the hull and domain shaders tessellate is drawn without a geometry
 *   shader and through one that passes the triangles through, which DXMT
 *   draws behind tessellation only as a pass-through: the pixel at the
 *   centre must be the pixel shader's red both times. The same patch then
 *   goes through layer_pass_gs of smoke/streamout.c, with a domain shader
 *   that writes a LAYER of 1, into a target of two slices: slice 1 must be
 *   red at the centre and slice 0 left as cleared. Then a triangle strip
 *   over the left half of the target goes through pass_gs_shdr and each
 *   geometry shader after it, with the default rasterizer state, which culls
 *   triangles turned away: one that passes its triangles through draws the
 *   vertex shader's red at the pixel read back, and each of the others
 *   draws what it computes: nothing there; for the one that moves the
 *   position to COLOR, the position's color; and for the one whose output
 *   drops the vertex shader's SV_ClipDistance of -1, the red D3D draws
 *   without the clip distance. A shader taken as a pass-through by mistake
 *   would draw the strip in red, or, for the last one, clip it away.
 * - swap-chain-detach: a swap chain released during DLL detach at process
 *   exit is destroyed and the process ends. The case creates a swap chain,
 *   presents once and hands it to smoke/dxmthold.c's DLL, which holds the
 *   last reference and releases it in its DLL_PROCESS_DETACH, and prints
 *   the case's result from there.
 *
 * The lifetime cases watch destruction through a tracker object attached
 * with SetPrivateDataInterface: the object holds the tracker's only
 * reference, so the tracker's last release is the object's destruction.
 *
 * Each case prints its results and one line saying whether its check
 * passed. The program exits 0 when it passed, 1 when it failed, 2 when
 * D3D11CreateDevice finds no adapter, 3 when it or another call the case
 * needs fails otherwise, and 4 without a known case name. */

#define COBJMACROS
#include <stdio.h>
#include <string.h>
#include <windows.h>
#include <initguid.h>
#include <d3d11_1.h>

#if defined(__aarch64__) && !defined(__arm64ec__)
#define ARCH "aarch64"
#elif defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

#define PREFIX "wine-build smoke test: " ARCH " "

/* The vertex shader of Wine's d3d11 test_stream_output_vs: POSITION,
   COLOR0 and COLOR1 passed through. Its header states 0x194 bytes. */
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

static const D3D11_INPUT_ELEMENT_DESC layout_desc[] = {
    {"POSITION", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0},
    {"COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 16, D3D11_INPUT_PER_VERTEX_DATA, 0},
    {"COLOR", 1, DXGI_FORMAT_R32G32B32A32_FLOAT, 0, 32, D3D11_INPUT_PER_VERTEX_DATA, 0},
};

static const D3D11_SO_DECLARATION_ENTRY so_decl[] = {
    {0, "POSITION", 0, 0, 4, 0},
};

/* The default vertex shader of Wine's d3d11 draw_quad: POSITION as
   SV_POSITION. */
static const DWORD quad_vs_code[] = {
    0x43425844, 0x4fb19b86, 0x955fa240, 0x1a630688, 0x24eb9db4, 0x00000001, 0x000001e0, 0x00000006,
    0x00000038, 0x00000084, 0x000000d0, 0x00000134, 0x00000178, 0x000001ac, 0x53414e58, 0x00000044,
    0x00000044, 0xfffe0200, 0x00000020, 0x00000024, 0x00240000, 0x00240000, 0x00240000, 0x00240000,
    0x00240000, 0xfffe0200, 0x0200001f, 0x80000005, 0x900f0000, 0x02000001, 0xc00f0000, 0x80e40000,
    0x0000ffff, 0x50414e58, 0x00000044, 0x00000044, 0xfffe0200, 0x00000020, 0x00000024, 0x00240000,
    0x00240000, 0x00240000, 0x00240000, 0x00240000, 0xfffe0200, 0x0200001f, 0x80000005, 0x900f0000,
    0x02000001, 0xc00f0000, 0x80e40000, 0x0000ffff, 0x396e6f41, 0x0000005c, 0x0000005c, 0xfffe0200,
    0x00000034, 0x00000028, 0x00240000, 0x00240000, 0x00240000, 0x00240000, 0x00240001, 0x00000000,
    0xfffe0200, 0x0200001f, 0x80000005, 0x900f0000, 0x04000004, 0xc0030000, 0x90ff0000, 0xa0e40000,
    0x90e40000, 0x02000001, 0xc00c0000, 0x90e40000, 0x0000ffff, 0x52444853, 0x0000003c, 0x00010040,
    0x0000000f, 0x0300005f, 0x001010f2, 0x00000000, 0x04000067, 0x001020f2, 0x00000000, 0x00000001,
    0x05000036, 0x001020f2, 0x00000000, 0x00101e46, 0x00000000, 0x0100003e, 0x4e475349, 0x0000002c,
    0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000f0f,
    0x49534f50, 0x4e4f4954, 0xababab00, 0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49,
};

/* A pixel shader of Wine's d3d11 tests: float4(1.0f, 0.0f, 0.0f, 0.5f). */
static const DWORD red_ps_code[] = {
    0x43425844, 0xb70eda74, 0xc9a7f982, 0xebc31bbf, 0x952a1360, 0x00000001, 0x00000168, 0x00000005,
    0x00000034, 0x0000008c, 0x000000e4, 0x00000124, 0x00000134, 0x53414e58, 0x00000050, 0x00000050,
    0xffff0200, 0x0000002c, 0x00000024, 0x00240000, 0x00240000, 0x00240000, 0x00240000, 0x00240000,
    0xffff0200, 0x05000051, 0xa00f0000, 0x3f800000, 0x00000000, 0x00000000, 0x3f000000, 0x02000001,
    0x800f0800, 0xa0e40000, 0x0000ffff, 0x396e6f41, 0x00000050, 0x00000050, 0xffff0200, 0x0000002c,
    0x00000024, 0x00240000, 0x00240000, 0x00240000, 0x00240000, 0x00240000, 0xffff0200, 0x05000051,
    0xa00f0000, 0x3f800000, 0x00000000, 0x00000000, 0x3f000000, 0x02000001, 0x800f0800, 0xa0e40000,
    0x0000ffff, 0x52444853, 0x00000038, 0x00000040, 0x0000000e, 0x03000065, 0x001020f2, 0x00000000,
    0x08000036, 0x001020f2, 0x00000000, 0x00004002, 0x3f800000, 0x00000000, 0x00000000, 0x3f000000,
    0x0100003e, 0x4e475349, 0x00000008, 0x00000000, 0x00000008, 0x4e47534f, 0x0000002c, 0x00000001,
    0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x0000000f, 0x545f5653,
    0x45475241, 0xabab0054,
};

/* The simple_gs of Wine's d3d11 test_clear_state: a point at POSITION
   becomes a quad 0.2 * w wide in a triangle strip. It is not a pass-through
   shader, so DXMT draws it with its mesh pipeline. */
static const DWORD point_quad_gs_code[] = {
    0x43425844, 0x000ee786, 0xc624c269, 0x885a5cbe, 0x444b3b1f, 0x00000001, 0x0000023c, 0x00000003,
    0x0000002c, 0x00000060, 0x00000094, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020,
    0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000f0f, 0x49534f50, 0x4e4f4954, 0xababab00,
    0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000001, 0x00000003,
    0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49, 0x52444853, 0x000001a0, 0x00020040,
    0x00000068, 0x0400005f, 0x002010f2, 0x00000001, 0x00000000, 0x02000068, 0x00000001, 0x0100085d,
    0x0100285c, 0x04000067, 0x001020f2, 0x00000000, 0x00000001, 0x0200005e, 0x00000004, 0x0f000032,
    0x00100032, 0x00000000, 0x80201ff6, 0x00000041, 0x00000000, 0x00000000, 0x00004002, 0x3dcccccd,
    0x3dcccccd, 0x00000000, 0x00000000, 0x00201046, 0x00000000, 0x00000000, 0x05000036, 0x00102032,
    0x00000000, 0x00100046, 0x00000000, 0x06000036, 0x001020c2, 0x00000000, 0x00201ea6, 0x00000000,
    0x00000000, 0x01000013, 0x05000036, 0x00102012, 0x00000000, 0x0010000a, 0x00000000, 0x0e000032,
    0x00100052, 0x00000000, 0x00201ff6, 0x00000000, 0x00000000, 0x00004002, 0x3dcccccd, 0x00000000,
    0x3dcccccd, 0x00000000, 0x00201106, 0x00000000, 0x00000000, 0x05000036, 0x00102022, 0x00000000,
    0x0010002a, 0x00000000, 0x06000036, 0x001020c2, 0x00000000, 0x00201ea6, 0x00000000, 0x00000000,
    0x01000013, 0x05000036, 0x00102012, 0x00000000, 0x0010000a, 0x00000000, 0x05000036, 0x00102022,
    0x00000000, 0x0010001a, 0x00000000, 0x06000036, 0x001020c2, 0x00000000, 0x00201ea6, 0x00000000,
    0x00000000, 0x01000013, 0x05000036, 0x00102032, 0x00000000, 0x00100086, 0x00000000, 0x06000036,
    0x001020c2, 0x00000000, 0x00201ea6, 0x00000000, 0x00000000, 0x01000013, 0x0100003e,
};

static const D3D11_INPUT_ELEMENT_DESC quad_layout_desc[] = {
    {"POSITION", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 0, D3D11_INPUT_PER_VERTEX_DATA, 0},
};

/* The hull and domain shaders of Wine's d3d11 test_quad_tessellation: a
   quad domain over a patch of four SV_POSITION control points, the
   tessellation factors from constant buffer 0 (float4 edges, float2
   inside), clockwise triangles; the domain shader interpolates control
   points 0 and 1, and 2 and 3, along u, and the two results along v. */
static const DWORD quad_hs_code[] = {
    0x43425844, 0x1ab30cc8, 0x94174771, 0x61f4cdd0, 0xa287f62c, 0x00000001, 0x000002b8, 0x00000004,
    0x00000030, 0x00000064, 0x00000098, 0x0000015c, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008,
    0x00000020, 0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x00000f0f, 0x505f5653, 0x5449534f,
    0x004e4f49, 0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000,
    0x00000003, 0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49, 0x47534350, 0x000000bc,
    0x00000006, 0x00000008, 0x00000098, 0x00000000, 0x0000000b, 0x00000003, 0x00000000, 0x00000e01,
    0x00000098, 0x00000001, 0x0000000b, 0x00000003, 0x00000001, 0x00000e01, 0x00000098, 0x00000002,
    0x0000000b, 0x00000003, 0x00000002, 0x00000e01, 0x00000098, 0x00000003, 0x0000000b, 0x00000003,
    0x00000003, 0x00000e01, 0x000000a6, 0x00000000, 0x0000000c, 0x00000003, 0x00000004, 0x00000e01,
    0x000000a6, 0x00000001, 0x0000000c, 0x00000003, 0x00000005, 0x00000e01, 0x545f5653, 0x46737365,
    0x6f746361, 0x56530072, 0x736e495f, 0x54656469, 0x46737365, 0x6f746361, 0xabab0072, 0x58454853,
    0x00000154, 0x00030050, 0x00000055, 0x01000071, 0x01002093, 0x01002094, 0x01001895, 0x01000896,
    0x01001897, 0x0100086a, 0x04000059, 0x00208e46, 0x00000000, 0x00000002, 0x01000073, 0x04000067,
    0x00102012, 0x00000000, 0x0000000b, 0x06000036, 0x00102012, 0x00000000, 0x0020800a, 0x00000000,
    0x00000000, 0x0100003e, 0x01000073, 0x04000067, 0x00102012, 0x00000001, 0x0000000c, 0x06000036,
    0x00102012, 0x00000001, 0x0020801a, 0x00000000, 0x00000000, 0x0100003e, 0x01000073, 0x04000067,
    0x00102012, 0x00000002, 0x0000000d, 0x06000036, 0x00102012, 0x00000002, 0x0020802a, 0x00000000,
    0x00000000, 0x0100003e, 0x01000073, 0x04000067, 0x00102012, 0x00000003, 0x0000000e, 0x06000036,
    0x00102012, 0x00000003, 0x0020803a, 0x00000000, 0x00000000, 0x0100003e, 0x01000073, 0x04000067,
    0x00102012, 0x00000004, 0x0000000f, 0x06000036, 0x00102012, 0x00000004, 0x0020800a, 0x00000000,
    0x00000001, 0x0100003e, 0x01000073, 0x04000067, 0x00102012, 0x00000005, 0x00000010, 0x06000036,
    0x00102012, 0x00000005, 0x0020801a, 0x00000000, 0x00000001, 0x0100003e,
};

static const DWORD quad_ds_code[] = {
    0x43425844, 0xeb6b7631, 0x07f5469e, 0xed0cbf4a, 0x7158b3a6, 0x00000001, 0x00000284, 0x00000004,
    0x00000030, 0x00000064, 0x00000128, 0x0000015c, 0x4e475349, 0x0000002c, 0x00000001, 0x00000008,
    0x00000020, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000f0f, 0x505f5653, 0x5449534f,
    0x004e4f49, 0x47534350, 0x000000bc, 0x00000006, 0x00000008, 0x00000098, 0x00000000, 0x0000000b,
    0x00000003, 0x00000000, 0x00000001, 0x00000098, 0x00000001, 0x0000000b, 0x00000003, 0x00000001,
    0x00000001, 0x00000098, 0x00000002, 0x0000000b, 0x00000003, 0x00000002, 0x00000001, 0x00000098,
    0x00000003, 0x0000000b, 0x00000003, 0x00000003, 0x00000001, 0x000000a6, 0x00000000, 0x0000000c,
    0x00000003, 0x00000004, 0x00000001, 0x000000a6, 0x00000001, 0x0000000c, 0x00000003, 0x00000005,
    0x00000001, 0x545f5653, 0x46737365, 0x6f746361, 0x56530072, 0x736e495f, 0x54656469, 0x46737365,
    0x6f746361, 0xabab0072, 0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000,
    0x00000001, 0x00000003, 0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49, 0x58454853,
    0x00000120, 0x00040050, 0x00000048, 0x01002093, 0x01001895, 0x0100086a, 0x0200005f, 0x0001c032,
    0x0400005f, 0x002190f2, 0x00000004, 0x00000000, 0x04000067, 0x001020f2, 0x00000000, 0x00000001,
    0x02000068, 0x00000002, 0x0a000000, 0x001000f2, 0x00000000, 0x80219e46, 0x00000041, 0x00000002,
    0x00000000, 0x00219e46, 0x00000003, 0x00000000, 0x09000032, 0x001000f2, 0x00000000, 0x0001c006,
    0x00100e46, 0x00000000, 0x00219e46, 0x00000002, 0x00000000, 0x0a000000, 0x001000f2, 0x00000001,
    0x80219e46, 0x00000041, 0x00000000, 0x00000000, 0x00219e46, 0x00000001, 0x00000000, 0x09000032,
    0x001000f2, 0x00000001, 0x0001c006, 0x00100e46, 0x00000001, 0x00219e46, 0x00000000, 0x00000000,
    0x08000000, 0x001000f2, 0x00000000, 0x00100e46, 0x00000000, 0x80100e46, 0x00000041, 0x00000001,
    0x08000032, 0x001020f2, 0x00000000, 0x0001c556, 0x00100e46, 0x00000000, 0x00100e46, 0x00000001,
    0x0100003e,
};

/* The chunks of the shaders of the pass-through case, assembled here as
   shader model 4.0, and 5.0 for the domain shader; build_shader makes the
   bytecode. Their shader chunks start with placeholders for their size and
   length, which build_shader fills in. */

/* SV_POSITION in register 0 and COLOR in register 1 */
#define COLOR_SIGNATURE(tag, position_mask, color_mask)                                                       \
    tag, 0x0000004c, 0x00000002, 0x00000008,                                                                  \
    0x00000038, 0x00000000, 0x00000001, 0x00000003, 0x00000000, position_mask,                               \
    0x00000044, 0x00000000, 0x00000000, 0x00000003, 0x00000001, color_mask,                                  \
    0x505f5653, 0x5449534f, 0x004e4f49, 0x4f4c4f43, 0xabab0052 /* "SV_POSITION", "COLOR" */
static const DWORD color_out_osgn[] = {COLOR_SIGNATURE(0x4e47534f, 0x0000000f, 0x0000000f)};
static const DWORD color_in_isgn[] = {COLOR_SIGNATURE(0x4e475349, 0x00000f0f, 0x00000f0f)};
static const DWORD color_ps_isgn[] = {COLOR_SIGNATURE(0x4e475349, 0x0000000f, 0x00000f0f)};

/* SV_POSITION in register 0, the signature of the domain shader's output */
static const DWORD position_in_isgn[] = {
    0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000001, 0x00000003,
    0x00000000, 0x00000f0f, 0x505f5653, 0x5449534f, 0x004e4f49,
};
static const DWORD position_out_osgn[] = {
    0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000001, 0x00000003,
    0x00000000, 0x0000000f, 0x505f5653, 0x5449534f, 0x004e4f49,
};

/* POSITION in register 0, the input signature of quad_vs_code */
static const DWORD position_vs_isgn[] = {
    0x4e475349, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003,
    0x00000000, 0x00000f0f, 0x49534f50, 0x4e4f4954, 0xababab00,
};

/* SV_TARGET in register 0, the output signature of red_ps_code */
static const DWORD target_ps_osgn[] = {
    0x4e47534f, 0x0000002c, 0x00000001, 0x00000008, 0x00000020, 0x00000000, 0x00000000, 0x00000003,
    0x00000000, 0x0000000f, 0x545f5653, 0x45475241, 0xabab0054,
};

/* SV_POSITION in register 0, COLOR in register 1 and SV_ClipDistance .x in
   register 2 */
#define CLIP_SIGNATURE(tag, position_mask, color_mask, clip_mask)                                             \
    tag, 0x00000074, 0x00000003, 0x00000008,                                                                  \
    0x00000050, 0x00000000, 0x00000001, 0x00000003, 0x00000000, position_mask,                               \
    0x0000005c, 0x00000000, 0x00000000, 0x00000003, 0x00000001, color_mask,                                  \
    0x00000064, 0x00000000, 0x00000002, 0x00000003, 0x00000002, clip_mask,                                   \
    0x505f5653, 0x5449534f, 0x004e4f49, 0x4f4c4f43, 0xabab0052,                                              \
    0x435f5653, 0x4470696c, 0x61747369, 0x0065636e /* "SV_POSITION", "COLOR", "SV_ClipDistance" */
static const DWORD clip_out_osgn[] = {CLIP_SIGNATURE(0x4e47534f, 0x0000000f, 0x0000000f, 0x00000e01)};
static const DWORD clip_in_isgn[] = {CLIP_SIGNATURE(0x4e475349, 0x00000f0f, 0x00000f0f, 0x00000101)};

/* SV_POSITION in register 0 and a uint LAYER .x in register 1, as in
   smoke/streamout.c: the output signature of layer_ds_shex and the input
   signature of layer_pass_gs_shdr */
#define LAYER_SIGNATURE(tag, position_mask, layer_mask)                                                       \
    tag, 0x0000004c, 0x00000002, 0x00000008,                                                                  \
    0x00000038, 0x00000000, 0x00000001, 0x00000003, 0x00000000, position_mask,                               \
    0x00000044, 0x00000000, 0x00000000, 0x00000001, 0x00000001, layer_mask,                                  \
    0x505f5653, 0x5449534f, 0x004e4f49, 0x4559414c, 0xabab0052 /* "SV_POSITION", "LAYER" */
static const DWORD layer_out_osgn[] = {LAYER_SIGNATURE(0x4e47534f, 0x0000000f, 0x00000e01)};
static const DWORD layer_gs_isgn[] = {LAYER_SIGNATURE(0x4e475349, 0x00000f0f, 0x00000101)};

/* SV_POSITION in register 0 and SV_RenderTargetArrayIndex .x in register 1 */
static const DWORD layer_gs_osgn[] = {
    0x4e47534f, 0x00000060, 0x00000002, 0x00000008,
    0x00000038, 0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x0000000f,
    0x00000044, 0x00000000, 0x00000004, 0x00000001, 0x00000001, 0x00000e01,
    0x505f5653, 0x5449534f, 0x004e4f49,                         /* "SV_POSITION" */
    0x525f5653, 0x65646e65, 0x72615472, 0x41746567, 0x79617272, 0x65646e49, 0xabab0078, /* "SV_RenderTargetArrayIndex" */
};

/* layer_pass_gs of smoke/streamout.c, the same chunks, which build_shader
   makes the same bytecode of: the triangle passed through, LAYER as
   SV_RenderTargetArrayIndex */
#define LAYER_PASS_VERTEX(v)                                                                                 \
    0x06000036, 0x001020f2, 0x00000000, 0x00201e46, v, 0x00000000, /* mov o0, v[v][0] */                   \
    0x06000036, 0x00102012, 0x00000001, 0x0020100a, v, 0x00000001, /* mov o1.x, v[v][1].x */               \
    0x01000013                                                     /* emit */
static const DWORD layer_pass_gs_shdr[] = {
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

/* The shader chunk of quad_ds_code with a LAYER of 1 in o1.x */
static const DWORD layer_ds_shex[] = {
    0x58454853, 0, 0x00040050, 0,
    0x01002093, 0x01001895, 0x0100086a, 0x0200005f, 0x0001c032, 0x0400005f, 0x002190f2, 0x00000004,
    0x00000000, 0x04000067, 0x001020f2, 0x00000000, 0x00000001,
    0x03000065, 0x00102012, 0x00000001,                         /* dcl_output o1.x */
    0x02000068, 0x00000002, 0x0a000000, 0x001000f2, 0x00000000, 0x80219e46, 0x00000041, 0x00000002,
    0x00000000, 0x00219e46, 0x00000003, 0x00000000, 0x09000032, 0x001000f2, 0x00000000, 0x0001c006,
    0x00100e46, 0x00000000, 0x00219e46, 0x00000002, 0x00000000, 0x0a000000, 0x001000f2, 0x00000001,
    0x80219e46, 0x00000041, 0x00000000, 0x00000000, 0x00219e46, 0x00000001, 0x00000000, 0x09000032,
    0x001000f2, 0x00000001, 0x0001c006, 0x00100e46, 0x00000001, 0x00219e46, 0x00000000, 0x00000000,
    0x08000000, 0x001000f2, 0x00000000, 0x00100e46, 0x00000000, 0x80100e46, 0x00000041, 0x00000001,
    0x08000032, 0x001020f2, 0x00000000, 0x0001c556, 0x00100e46, 0x00000000, 0x00100e46, 0x00000001,
    0x05000036, 0x00102012, 0x00000001, 0x00004001, 0x00000001, /* mov o1.x, l(1) */
    0x0100003e,                                                 /* ret */
};
/* The input and patch constant signature chunks of quad_ds_code */
#define QUAD_DS_ISGN ((const DWORD *)quad_ds_code + 12)
#define QUAD_DS_ISGN_WORDS 13
#define QUAD_DS_PCSG ((const DWORD *)quad_ds_code + 25)
#define QUAD_DS_PCSG_WORDS 49

/* The position passed through, COLOR red and SV_ClipDistance -1, which clips
   every vertex */
static const DWORD clip_vs_shdr[] = {
    0x52444853, 0, 0x00010040, 0,
    0x0300005f, 0x001010f2, 0x00000000,                         /* dcl_input v0.xyzw */
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */
    0x03000065, 0x001020f2, 0x00000001,                         /* dcl_output o1.xyzw */
    0x04000067, 0x00102012, 0x00000002, 0x00000002,             /* dcl_output_siv o2.x, clip_distance */
    0x05000036, 0x001020f2, 0x00000000, 0x00101e46, 0x00000000, /* mov o0, v0 */
    0x08000036, 0x001020f2, 0x00000001, 0x00004002, 0x3f800000, 0x00000000, 0x00000000, 0x3f800000,
                                                                /* mov o1, l(1.0, 0.0, 0.0, 1.0) */
    0x05000036, 0x00102012, 0x00000002, 0x00004001, 0xbf800000, /* mov o2.x, l(-1.0) */
    0x0100003e,                                                 /* ret */
};

/* The position passed through and COLOR red */
static const DWORD color_vs_shdr[] = {
    0x52444853, 0, 0x00010040, 0,
    0x0300005f, 0x001010f2, 0x00000000,                         /* dcl_input v0.xyzw */
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */
    0x03000065, 0x001020f2, 0x00000001,                         /* dcl_output o1.xyzw */
    0x05000036, 0x001020f2, 0x00000000, 0x00101e46, 0x00000000, /* mov o0, v0 */
    0x08000036, 0x001020f2, 0x00000001, 0x00004002, 0x3f800000, 0x00000000, 0x00000000, 0x3f800000,
                                                                /* mov o1, l(1.0, 0.0, 0.0, 1.0) */
    0x0100003e,                                                 /* ret */
};

/* COLOR as the target's color */
static const DWORD color_ps_shdr[] = {
    0x52444853, 0, 0x00000040, 0,
    0x03001062, 0x001010f2, 0x00000001,                         /* dcl_input_ps linear v1.xyzw */
    0x03000065, 0x001020f2, 0x00000000,                         /* dcl_output o0.xyzw */
    0x05000036, 0x001020f2, 0x00000000, 0x00101e46, 0x00000001, /* mov o0, v1 */
    0x0100003e,                                                 /* ret */
};

/* The geometry shaders of a triangle with SV_POSITION and COLOR in and out:
   one that passes its triangle through, the vertices emitted in order, and
   shaders that differ from it in one point each, which DXMT must not take
   as a pass-through. Each draws the vertex shader's red at the pixel the
   case reads back only if it is taken as one. */
#define COLOR_GS_HEAD(topology)                                                                                \
    0x52444853, 0, 0x00020040, 0,                                                                             \
    0x05000061, 0x002010f2, 0x00000003, 0x00000000, 0x00000001, /* dcl_input_siv v[3][0].xyzw, position */   \
    0x0400005f, 0x002010f2, 0x00000003, 0x00000001,             /* dcl_input v[3][1].xyzw */                 \
    0x0100185d,                                                 /* dcl_inputprimitive triangle */            \
    topology,                                                   /* dcl_outputtopology */                     \
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */       \
    0x03000065, 0x001020f2, 0x00000001,                         /* dcl_output o1.xyzw */                     \
    0x0200005e, 0x00000003                                      /* dcl_maxout 3 */
#define SM4_TRIANGLESTRIP 0x0100285c /* dcl_outputtopology trianglestrip */
#define SM4_POINTLIST 0x0100085c     /* dcl_outputtopology pointlist */
#define SM4_EMIT 0x01000013          /* emit */
#define SM4_RET 0x0100003e           /* ret */
#define MOV_POSITION(v) 0x06000036, 0x001020f2, 0x00000000, 0x00201e46, v, 0x00000000 /* mov o0, v[v][0] */
#define MOV_COLOR(v) 0x06000036, 0x001020f2, 0x00000001, 0x00201e46, v, 0x00000001    /* mov o1, v[v][1] */
#define PASS_VERTEX(v) MOV_POSITION(v), MOV_COLOR(v), SM4_EMIT

static const DWORD pass_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), PASS_VERTEX(0), PASS_VERTEX(1), PASS_VERTEX(2), SM4_RET,
};
/* The vertices in the other order, which turns the triangles away */
static const DWORD order_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), PASS_VERTEX(0), PASS_VERTEX(2), PASS_VERTEX(1), SM4_RET,
};
/* The outputs written for the first vertex only and emitted three times:
   they are undefined after the first emit */
static const DWORD first_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), PASS_VERTEX(0), SM4_EMIT, SM4_EMIT, SM4_RET,
};
/* Two vertices, no triangle */
static const DWORD part_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), PASS_VERTEX(0), PASS_VERTEX(1), SM4_RET,
};
/* The vertices as points */
static const DWORD points_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_POINTLIST), PASS_VERTEX(0), PASS_VERTEX(1), PASS_VERTEX(2), SM4_RET,
};
/* A constant position off the target */
#define CONSTANT_VERTEX(v)                                                                                     \
    0x08000036, 0x001020f2, 0x00000000, 0x00004002, 0x40000000, 0x40000000, 0x3f000000, 0x3f800000,          \
                                                                /* mov o0, l(2.0, 2.0, 0.5, 1.0) */          \
    MOV_COLOR(v), SM4_EMIT
static const DWORD constant_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), CONSTANT_VERTEX(0), CONSTANT_VERTEX(1), CONSTANT_VERTEX(2), SM4_RET,
};
/* x and y of the position swapped */
#define SWIZZLE_VERTEX(v)                                                                                      \
    0x06000036, 0x001020f2, 0x00000000, 0x00201e16, v, 0x00000000, /* mov o0, v[v][0].yxzw */                \
    MOV_COLOR(v), SM4_EMIT
static const DWORD swizzle_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), SWIZZLE_VERTEX(0), SWIZZLE_VERTEX(1), SWIZZLE_VERTEX(2), SM4_RET,
};
/* x of the position negated */
#define NEGATE_VERTEX(v)                                                                                       \
    0x07000036, 0x00102012, 0x00000000, 0x8020100a, 0x00000041, v, 0x00000000, /* mov o0.x, -v[v][0].x */   \
    0x06000036, 0x001020e2, 0x00000000, 0x00201e56, v, 0x00000000,             /* mov o0.yzw, v[v][0].yyzw */ \
    MOV_COLOR(v), SM4_EMIT
static const DWORD negate_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), NEGATE_VERTEX(0), NEGATE_VERTEX(1), NEGATE_VERTEX(2), SM4_RET,
};
/* The position saturated, which puts every vertex of the half quad at x 0 */
#define SATURATE_VERTEX(v)                                                                                     \
    0x06002036, 0x001020f2, 0x00000000, 0x00201e46, v, 0x00000000, /* mov_sat o0, v[v][0] */                 \
    MOV_COLOR(v), SM4_EMIT
static const DWORD saturate_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), SATURATE_VERTEX(0), SATURATE_VERTEX(1), SATURATE_VERTEX(2), SM4_RET,
};
/* COLOR from the position's register: the pixel's color is the position */
#define REGISTER_VERTEX(v)                                                                                     \
    MOV_POSITION(v), 0x06000036, 0x001020f2, 0x00000001, 0x00201e46, v, 0x00000000, /* mov o1, v[v][0] */   \
    SM4_EMIT
static const DWORD register_gs_shdr[] = {
    COLOR_GS_HEAD(SM4_TRIANGLESTRIP), REGISTER_VERTEX(0), REGISTER_VERTEX(1), REGISTER_VERTEX(2), SM4_RET,
};

/* A geometry shader that passes the domain shader's triangles through */
static const DWORD position_pass_gs_shdr[] = {
    0x52444853, 0, 0x00020040, 0,
    0x05000061, 0x002010f2, 0x00000003, 0x00000000, 0x00000001, /* dcl_input_siv v[3][0].xyzw, position */
    0x0100185d,                                                 /* dcl_inputprimitive triangle */
    SM4_TRIANGLESTRIP,
    0x04000067, 0x001020f2, 0x00000000, 0x00000001,             /* dcl_output_siv o0.xyzw, position */
    0x0200005e, 0x00000003,                                     /* dcl_maxout 3 */
    MOV_POSITION(0), SM4_EMIT, MOV_POSITION(1), SM4_EMIT, MOV_POSITION(2), SM4_EMIT, SM4_RET,
};

/* The private data key the trackers are attached under. */
static const GUID tracker_key = {0x5bd6c0a4, 0x1d0e, 0x4a4b, {0x9b, 0x3e, 0x6a, 0x71, 0x2c, 0x84, 0x10, 0x93}};

struct tracker
{
    IUnknown IUnknown_iface;
    LONG refcount;
    /* The position of its last release among all trackers' (1, 2, ...),
       0 while it is referenced. */
    int destroyed;
};

static int destroy_count;

static HRESULT STDMETHODCALLTYPE tracker_QueryInterface(IUnknown *iface, REFIID iid, void **out)
{
    static const GUID iid_unknown = {0x00000000, 0x0000, 0x0000, {0xc0, 0, 0, 0, 0, 0, 0, 0x46}};

    if (!memcmp(iid, &iid_unknown, sizeof(GUID)))
    {
        IUnknown_AddRef(iface);
        *out = iface;
        return S_OK;
    }
    *out = NULL;
    return E_NOINTERFACE;
}

static ULONG STDMETHODCALLTYPE tracker_AddRef(IUnknown *iface)
{
    struct tracker *tracker = (struct tracker *)iface;

    return InterlockedIncrement(&tracker->refcount);
}

static ULONG STDMETHODCALLTYPE tracker_Release(IUnknown *iface)
{
    struct tracker *tracker = (struct tracker *)iface;
    LONG refcount = InterlockedDecrement(&tracker->refcount);

    if (!refcount)
        tracker->destroyed = ++destroy_count;
    return refcount;
}

static IUnknownVtbl tracker_vtbl = {tracker_QueryInterface, tracker_AddRef, tracker_Release};

static IUnknown *tracker_init(struct tracker *tracker)
{
    tracker->IUnknown_iface.lpVtbl = &tracker_vtbl;
    tracker->refcount = 1;
    tracker->destroyed = 0;
    return &tracker->IUnknown_iface;
}

/* tracker_hand_over: after SetPrivateDataInterface returned hr, drop the
   program's own reference to the tracker, and return whether the object
   now holds the only one. */
static int tracker_hand_over(struct tracker *tracker, HRESULT hr)
{
    IUnknown_Release(&tracker->IUnknown_iface);
    return SUCCEEDED(hr) && tracker->refcount == 1;
}

/* fail_now: end the process with status 1 after a failed lifetime check.
   DXMT can have used freed memory by then, so no DLL detach code may run. */
static void fail_now(const char *name)
{
    printf(PREFIX "%s check FAILED\n", name);
    fflush(stdout);
    TerminateProcess(GetCurrentProcess(), 1);
}

static int check_bytecode(ID3D11Device *device, ID3D11DeviceContext *context)
{
    const UINT full = sizeof(vs_code), cut = sizeof(vs_code) - 16;
    ID3D11InputLayout *il_full = NULL, *il_cut = NULL;
    ID3D11GeometryShader *gs_full = NULL, *gs_cut = NULL;
    HRESULT hr_full, hr_cut, hr_gs_full, hr_gs_cut;
    UINT stride = 16;
    int ok;

    hr_full = ID3D11Device_CreateInputLayout(device, layout_desc, 3, vs_code, full, &il_full);
    hr_cut = ID3D11Device_CreateInputLayout(device, layout_desc, 3, vs_code, cut, &il_cut);
    hr_gs_full = ID3D11Device_CreateGeometryShaderWithStreamOutput(
        device, vs_code, full, so_decl, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM, NULL, &gs_full);
    hr_gs_cut = ID3D11Device_CreateGeometryShaderWithStreamOutput(
        device, vs_code, cut, so_decl, 1, &stride, 1, D3D11_SO_NO_RASTERIZED_STREAM, NULL, &gs_cut);
    ok = SUCCEEDED(hr_full) && FAILED(hr_cut) && SUCCEEDED(hr_gs_full) && FAILED(hr_gs_cut);

    printf(PREFIX "bytecode: input layout full %#lx short %#lx, stream output full %#lx short %#lx\n",
           (unsigned long)hr_full, (unsigned long)hr_cut, (unsigned long)hr_gs_full, (unsigned long)hr_gs_cut);
    printf(PREFIX "bytecode check %s\n", ok ? "ok" : "FAILED");

    if (il_full)
        ID3D11InputLayout_Release(il_full);
    if (il_cut)
        ID3D11InputLayout_Release(il_cut);
    if (gs_full)
        ID3D11GeometryShader_Release(gs_full);
    if (gs_cut)
        ID3D11GeometryShader_Release(gs_cut);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return ok ? 0 : 1;
}

static int check_swap_state(ID3D11Device *device, ID3D11DeviceContext *context)
{
    ID3DDeviceContextState *state, *previous = NULL;
    D3D_FEATURE_LEVEL level = D3D_FEATURE_LEVEL_11_0;
    ID3D11DeviceContext1 *context1;
    ID3D11Device1 *device1;
    struct tracker tracker;
    int alive, ok;
    HRESULT hr;

    if (FAILED(hr = ID3D11Device_QueryInterface(device, &IID_ID3D11Device1, (void **)&device1))
        || FAILED(hr = ID3D11DeviceContext_QueryInterface(context, &IID_ID3D11DeviceContext1, (void **)&context1)))
    {
        printf(PREFIX "swap-state: no ID3D11Device1 or ID3D11DeviceContext1, %#lx\n", (unsigned long)hr);
        return 3;
    }
    hr = ID3D11Device1_CreateDeviceContextState(device1, 0, &level, 1, D3D11_SDK_VERSION, &IID_ID3D11Device1,
                                                NULL, &state);
    if (FAILED(hr))
    {
        printf(PREFIX "swap-state: CreateDeviceContextState failed, %#lx\n", (unsigned long)hr);
        return 3;
    }
    hr = ID3DDeviceContextState_SetPrivateDataInterface(state, &tracker_key, tracker_init(&tracker));
    if (!tracker_hand_over(&tracker, hr))
    {
        printf(PREFIX "swap-state: SetPrivateDataInterface %#lx, tracker references %ld\n", (unsigned long)hr,
               tracker.refcount);
        return 3;
    }

    /* Install the state, which the context then holds, and release the
       program's reference: the installed state is the only holder. */
    ID3D11DeviceContext1_SwapDeviceContextState(context1, state, &previous);
    ID3DDeviceContextState_Release(state);
    if (!previous || tracker.destroyed)
    {
        printf(PREFIX "swap-state: first swap returned %p, state destroyed %d\n", (void *)previous,
               tracker.destroyed);
        return 3;
    }

    /* Swap in the installed state through that released pointer. */
    ID3D11DeviceContext1_SwapDeviceContextState(context1, state, NULL);
    alive = !tracker.destroyed;
    printf(PREFIX "swap-state: installed state %s by swapping it in again\n", alive ? "kept" : "destroyed");
    if (!alive)
    {
        fail_now("swap-state");
        return 1;
    }

    /* Swapping the previous state back in releases the last reference, which
       shows that the tracker records the state's destruction. */
    ID3D11DeviceContext1_SwapDeviceContextState(context1, previous, NULL);
    ok = tracker.destroyed != 0;
    printf(PREFIX "swap-state: state %s when swapped out\n", ok ? "destroyed" : "kept");
    printf(PREFIX "swap-state check %s\n", ok ? "ok" : "FAILED");

    ID3DDeviceContextState_Release(previous);
    ID3D11DeviceContext1_Release(context1);
    ID3D11Device1_Release(device1);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return ok ? 0 : 1;
}

static int check_deferred(ID3D11Device *device, ID3D11DeviceContext *context)
{
    struct tracker device_tracker, deferred_tracker;
    ID3D11DeviceContext *deferred;
    int alive, ok;
    HRESULT hr;

    if (FAILED(hr = ID3D11Device_CreateDeferredContext(device, 0, &deferred)))
    {
        printf(PREFIX "deferred: CreateDeferredContext failed, %#lx\n", (unsigned long)hr);
        return 3;
    }
    hr = ID3D11Device_SetPrivateDataInterface(device, &tracker_key, tracker_init(&device_tracker));
    if (!tracker_hand_over(&device_tracker, hr))
    {
        printf(PREFIX "deferred: device SetPrivateDataInterface %#lx, tracker references %ld\n",
               (unsigned long)hr, device_tracker.refcount);
        return 3;
    }
    hr = ID3D11DeviceContext_SetPrivateDataInterface(deferred, &tracker_key, tracker_init(&deferred_tracker));
    if (!tracker_hand_over(&deferred_tracker, hr))
    {
        printf(PREFIX "deferred: context SetPrivateDataInterface %#lx, tracker references %ld\n",
               (unsigned long)hr, deferred_tracker.refcount);
        return 3;
    }

    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    alive = !device_tracker.destroyed;
    ID3D11DeviceContext_Release(deferred);
    ok = alive && deferred_tracker.destroyed && device_tracker.destroyed
         && deferred_tracker.destroyed < device_tracker.destroyed;

    printf(PREFIX "deferred: device %s after its release; destroyed in order: device %d, deferred context %d\n",
           alive ? "kept" : "destroyed", device_tracker.destroyed, deferred_tracker.destroyed);
    if (!ok)
    {
        fail_now("deferred");
        return 1;
    }
    printf(PREFIX "deferred check ok\n");
    return 0;
}

#define BIAS_SIZE 64
/* The pixel read back, near the centre, inside every quad drawn. */
#define BIAS_X 31
#define BIAS_Y 31
#define D24_MAX 16777215.0

struct bias_target
{
    ID3D11Texture2D *color, *color_staging, *depth, *depth_staging;
    ID3D11RenderTargetView *rtv;
    ID3D11DepthStencilView *dsv;
};

static HRESULT create_bias_texture(ID3D11Device *device, DXGI_FORMAT format, UINT bind, BOOL staging,
                                   ID3D11Texture2D **texture)
{
    D3D11_TEXTURE2D_DESC desc = {0};

    desc.Width = BIAS_SIZE;
    desc.Height = BIAS_SIZE;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = format;
    desc.SampleDesc.Count = 1;
    desc.Usage = staging ? D3D11_USAGE_STAGING : D3D11_USAGE_DEFAULT;
    desc.BindFlags = staging ? 0 : bind;
    desc.CPUAccessFlags = staging ? D3D11_CPU_ACCESS_READ : 0;
    return ID3D11Device_CreateTexture2D(device, &desc, NULL, texture);
}

/* read_bias_pixel: copy both targets to their staging textures and return
   the 24-bit depth and the color of the pixel read back, or FALSE when a
   Map fails. */
static BOOL read_bias_pixel(ID3D11DeviceContext *context, struct bias_target *target, UINT *depth, UINT *color)
{
    D3D11_MAPPED_SUBRESOURCE map;

    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)target->depth_staging,
                                     (ID3D11Resource *)target->depth);
    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)target->color_staging,
                                     (ID3D11Resource *)target->color);
    if (FAILED(ID3D11DeviceContext_Map(context, (ID3D11Resource *)target->depth_staging, 0, D3D11_MAP_READ, 0, &map)))
        return FALSE;
    /* D24_UNORM_S8_UINT: depth in the low 24 bits, stencil in the high 8 */
    *depth = ((const UINT *)((const BYTE *)map.pData + BIAS_Y * map.RowPitch))[BIAS_X] & 0xffffff;
    ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)target->depth_staging, 0);
    if (FAILED(ID3D11DeviceContext_Map(context, (ID3D11Resource *)target->color_staging, 0, D3D11_MAP_READ, 0, &map)))
        return FALSE;
    *color = ((const UINT *)((const BYTE *)map.pData + BIAS_Y * map.RowPitch))[BIAS_X];
    ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)target->color_staging, 0);
    return TRUE;
}

/* draw_bias: clear, then for each of the draws DepthBias values set a
   rasterizer state with it and draw count vertices of the bound input, all
   in one render pass, and return the pixel's depth and color. */
static BOOL draw_bias(ID3D11Device *device, ID3D11DeviceContext *context, struct bias_target *target,
                      const int *depth_biases, UINT draws, float slope_scaled_depth_bias, UINT count, UINT *depth,
                      UINT *color)
{
    static const float zero[4];
    D3D11_RASTERIZER_DESC rs_desc = {0};
    ID3D11RasterizerState *rs[2] = {0};
    BOOL ok = TRUE;
    UINT i;

    if (draws > ARRAYSIZE(rs))
        return FALSE;
    rs_desc.FillMode = D3D11_FILL_SOLID;
    rs_desc.CullMode = D3D11_CULL_NONE;
    rs_desc.SlopeScaledDepthBias = slope_scaled_depth_bias;
    rs_desc.DepthClipEnable = TRUE;
    for (i = 0; i < draws && ok; i++)
    {
        rs_desc.DepthBias = depth_biases[i];
        ok = SUCCEEDED(ID3D11Device_CreateRasterizerState(device, &rs_desc, &rs[i]));
    }
    if (ok)
    {
        ID3D11DeviceContext_ClearRenderTargetView(context, target->rtv, zero);
        ID3D11DeviceContext_ClearDepthStencilView(context, target->dsv, D3D11_CLEAR_DEPTH | D3D11_CLEAR_STENCIL,
                                                  1.0f, 0);
        for (i = 0; i < draws; i++)
        {
            ID3D11DeviceContext_RSSetState(context, rs[i]);
            ID3D11DeviceContext_Draw(context, count, 0);
        }
        ID3D11DeviceContext_RSSetState(context, NULL);
        ok = read_bias_pixel(context, target, depth, color);
    }
    for (i = 0; i < draws; i++)
        if (rs[i])
            ID3D11RasterizerState_Release(rs[i]);
    return ok;
}

/* check_bias_result: print one draw's result and return whether its depth is
   within tolerance units of expected and its color is want_color. */
static int check_bias_result(const char *what, UINT depth, double expected, UINT tolerance, UINT color,
                             UINT want_color)
{
    UINT want = (UINT)(expected + 0.5);
    UINT diff = depth > want ? depth - want : want - depth;
    int ok = diff <= tolerance && (color & 0x00ffffff) == want_color;

    printf(PREFIX "depth-bias: %s: depth %#x, expected %#x (+-%u), color %#x %s\n", what, depth, want, tolerance,
           color, ok ? "ok" : "FAILED");
    return ok;
}

static int check_depth_bias(ID3D11Device *device, ID3D11DeviceContext *context)
{
    /* Triangle strips over the whole target: flat at 0.25, and sloped from
       0.375 at the bottom to 0.125 at the top. */
    static const float flat_quad[] = {
        -1.0f, -1.0f, 0.25f, -1.0f, 1.0f, 0.25f, 1.0f, -1.0f, 0.25f, 1.0f, 1.0f, 0.25f,
    };
    static const float sloped_quad[] = {
        -1.0f, -1.0f, 0.375f, -1.0f, 1.0f, 0.125f, 1.0f, -1.0f, 0.375f, 1.0f, 1.0f, 0.125f,
    };
    /* One point for the geometry shader; vs_code reads POSITION, COLOR and
       COLOR1. */
    static const float point[] = {
        0.0f, 0.0f, 0.25f, 1.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f,
    };
    /* The pixel shader's red with any alpha, and the cleared color. */
    static const UINT red = 0x0000ff, cleared = 0;
    static const int plus[] = {1000}, minus[] = {-1000}, none[] = {0}, plus_then_minus[] = {1000, -1000};
    D3D11_BUFFER_DESC buffer_desc = {0};
    D3D11_SUBRESOURCE_DATA data = {0};
    ID3D11Buffer *flat_vb = NULL, *sloped_vb = NULL, *point_vb = NULL;
    ID3D11InputLayout *quad_layout = NULL, *point_layout = NULL;
    ID3D11VertexShader *quad_vs = NULL, *point_vs = NULL;
    ID3D11GeometryShader *gs = NULL;
    ID3D11PixelShader *ps = NULL;
    struct bias_target target = {0};
    UINT quad_stride = 3 * sizeof(float), point_stride = 12 * sizeof(float), offset = 0;
    UINT depth, color, sloped_ref;
    D3D11_VIEWPORT vp = {0.0f, 0.0f, BIAS_SIZE, BIAS_SIZE, 0.0f, 1.0f};
    /* MaxDepthSlope of the sloped quad: 0.25 over BIAS_SIZE rows */
    const double slope = 0.25 / BIAS_SIZE;
    int ok = 1, ret = 3;
    HRESULT hr;

    buffer_desc.Usage = D3D11_USAGE_DEFAULT;
    buffer_desc.BindFlags = D3D11_BIND_VERTEX_BUFFER;
    buffer_desc.ByteWidth = sizeof(flat_quad);
    data.pSysMem = flat_quad;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &flat_vb)))
        goto done;
    data.pSysMem = sloped_quad;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &sloped_vb)))
        goto done;
    buffer_desc.ByteWidth = sizeof(point);
    data.pSysMem = point;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &point_vb)))
        goto done;
    if (FAILED(hr = ID3D11Device_CreateVertexShader(device, quad_vs_code, sizeof(quad_vs_code), NULL, &quad_vs))
        || FAILED(hr = ID3D11Device_CreateVertexShader(device, vs_code, sizeof(vs_code), NULL, &point_vs))
        || FAILED(hr = ID3D11Device_CreateGeometryShader(device, point_quad_gs_code, sizeof(point_quad_gs_code),
                                                         NULL, &gs))
        || FAILED(hr = ID3D11Device_CreatePixelShader(device, red_ps_code, sizeof(red_ps_code), NULL, &ps))
        || FAILED(hr = ID3D11Device_CreateInputLayout(device, quad_layout_desc, 1, quad_vs_code,
                                                      sizeof(quad_vs_code), &quad_layout))
        || FAILED(hr = ID3D11Device_CreateInputLayout(device, layout_desc, 3, vs_code, sizeof(vs_code),
                                                      &point_layout)))
        goto done;
    if (FAILED(hr = create_bias_texture(device, DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_BIND_RENDER_TARGET, FALSE,
                                        &target.color))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_R8G8B8A8_UNORM, 0, TRUE, &target.color_staging))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_D24_UNORM_S8_UINT, D3D11_BIND_DEPTH_STENCIL, FALSE,
                                           &target.depth))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_D24_UNORM_S8_UINT, 0, TRUE, &target.depth_staging))
        || FAILED(hr = ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)target.color, NULL,
                                                           &target.rtv))
        || FAILED(hr = ID3D11Device_CreateDepthStencilView(device, (ID3D11Resource *)target.depth, NULL,
                                                           &target.dsv)))
        goto done;

    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &target.rtv, target.dsv);
    ID3D11DeviceContext_RSSetViewports(context, 1, &vp);
    ID3D11DeviceContext_IASetInputLayout(context, quad_layout);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
    ID3D11DeviceContext_VSSetShader(context, quad_vs, NULL, 0);
    ID3D11DeviceContext_PSSetShader(context, ps, NULL, 0);
    hr = E_FAIL;

    /* A constant bias of each sign, with a pixel shader. */
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &flat_vb, &quad_stride, &offset);
    if (!draw_bias(device, context, &target, plus, 1, 0.0f, 4, &depth, &color))
        goto done;
    ok &= check_bias_result("pixel shader, DepthBias 1000", depth, 0.25 * D24_MAX + 1000, 1, color, red);
    if (!draw_bias(device, context, &target, minus, 1, 0.0f, 4, &depth, &color))
        goto done;
    ok &= check_bias_result("pixel shader, DepthBias -1000", depth, 0.25 * D24_MAX - 1000, 1, color, red);

    /* +1000, then -1000 in the same render pass, which switches the pipeline
       and the bias between the draws: under LESS the second draw's depth is
       the one left. */
    if (!draw_bias(device, context, &target, plus_then_minus, 2, 0.0f, 4, &depth, &color))
        goto done;
    ok &= check_bias_result("pixel shader, DepthBias 1000 then -1000 in one pass", depth, 0.25 * D24_MAX - 1000, 1,
                            color, red);

    /* Both terms, of opposite signs, on the sloped quad; the reference is
       the same draw without a bias. */
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &sloped_vb, &quad_stride, &offset);
    if (!draw_bias(device, context, &target, none, 1, 0.0f, 4, &sloped_ref, &color))
        goto done;
    if (!draw_bias(device, context, &target, plus, 1, -2.0f, 4, &depth, &color))
        goto done;
    ok &= check_bias_result("pixel shader, DepthBias 1000, SlopeScaledDepthBias -2", depth,
                            sloped_ref + 1000 - 2.0 * slope * D24_MAX, 3, color, red);

    /* The same without a pixel shader: depth only, the color stays
       cleared. */
    ID3D11DeviceContext_PSSetShader(context, NULL, NULL, 0);
    if (!draw_bias(device, context, &target, plus, 1, -2.0f, 4, &depth, &color))
        goto done;
    ok &= check_bias_result("no pixel shader, DepthBias 1000, SlopeScaledDepthBias -2", depth,
                            sloped_ref + 1000 - 2.0 * slope * D24_MAX, 3, color, cleared);

    /* Through the geometry shader, with the pixel shader. */
    ID3D11DeviceContext_PSSetShader(context, ps, NULL, 0);
    ID3D11DeviceContext_IASetInputLayout(context, point_layout);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &point_vb, &point_stride, &offset);
    ID3D11DeviceContext_VSSetShader(context, point_vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, gs, NULL, 0);
    if (!draw_bias(device, context, &target, plus, 1, 0.0f, 1, &depth, &color))
        goto done;
    ok &= check_bias_result("geometry shader, DepthBias 1000", depth, 0.25 * D24_MAX + 1000, 1, color, red);
    ID3D11DeviceContext_GSSetShader(context, NULL, NULL, 0);

    printf(PREFIX "depth-bias check %s\n", ok ? "ok" : "FAILED");
    ret = ok ? 0 : 1;

done:
    if (ret == 3)
        printf(PREFIX "depth-bias: setup or readback failed, %#lx\n", (unsigned long)hr);
    if (target.dsv)
        ID3D11DepthStencilView_Release(target.dsv);
    if (target.rtv)
        ID3D11RenderTargetView_Release(target.rtv);
    if (target.depth_staging)
        ID3D11Texture2D_Release(target.depth_staging);
    if (target.depth)
        ID3D11Texture2D_Release(target.depth);
    if (target.color_staging)
        ID3D11Texture2D_Release(target.color_staging);
    if (target.color)
        ID3D11Texture2D_Release(target.color);
    if (point_layout)
        ID3D11InputLayout_Release(point_layout);
    if (quad_layout)
        ID3D11InputLayout_Release(quad_layout);
    if (ps)
        ID3D11PixelShader_Release(ps);
    if (gs)
        ID3D11GeometryShader_Release(gs);
    if (point_vs)
        ID3D11VertexShader_Release(point_vs);
    if (quad_vs)
        ID3D11VertexShader_Release(quad_vs);
    if (point_vb)
        ID3D11Buffer_Release(point_vb);
    if (sloped_vb)
        ID3D11Buffer_Release(sloped_vb);
    if (flat_vb)
        ID3D11Buffer_Release(flat_vb);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return ret;
}

/* The units of the index cases sit side by side on the middle row of a
   BIAS_SIZE target, unit u centred at x = -0.75 + 0.5 * u. */
#define INDEX_UNITS 4
#define INDEX_UNIT_X(u) (-0.75f + 0.5f * (u))
#define INDEX_UNIT_PIXEL(u) (BIAS_SIZE / 8 + (u) * BIAS_SIZE / 4)

/* The index buffers in units; each unit u stands for the vertices
   u * n to u * n + n - 1 of a unit of n vertices. */
static const UINT index16_units[] = {2, 1, 0, 0};
static const UINT index32_units[] = {0, 3};

struct index_draw
{
    /* DXGI_FORMAT_UNKNOWN for a non-indexed draw */
    DXGI_FORMAT format;
    /* The first vertex or index, in units */
    UINT start;
};

/* Each pass draws two units and must light exactly those (the mask). On
   the pipeline of the first draw, the second one would read the vertices
   of unit 0 instead of those in the comment: DrawIndexed takes StartIndex
   for its first vertex, Draw reads the indices of index16_units[3], and
   indices of the other width read as these give unit 0 too, except in the
   last pass with patches, which then read vertices past the buffer. */
static const struct
{
    const char *what;
    struct index_draw draws[2];
    UINT lit;
} index_passes[] = {
    /* 0 instead of 2 */
    {"Draw, then DrawIndexed", {{DXGI_FORMAT_UNKNOWN, 0}, {DXGI_FORMAT_R16_UINT, 0}}, 0x5},
    /* 0 instead of 3 */
    {"R16, then R32 indices", {{DXGI_FORMAT_R16_UINT, 1}, {DXGI_FORMAT_R32_UINT, 1}}, 0xa},
    /* 0 instead of 3 */
    {"DrawIndexed, then Draw", {{DXGI_FORMAT_R16_UINT, 0}, {DXGI_FORMAT_UNKNOWN, 3}}, 0xc},
    /* 0 instead of 1 */
    {"R32, then R16 indices", {{DXGI_FORMAT_R32_UINT, 1}, {DXGI_FORMAT_R16_UINT, 1}}, 0xa},
};

/* check_index_draws: with the shaders, input layout, topology and vertex
   buffer of INDEX_UNITS units of n vertices each bound, run index_passes
   and return 0 when each lit the units it should, 1 when one did not and
   3 when the setup or a readback failed. */
static int check_index_draws(ID3D11Device *device, ID3D11DeviceContext *context, const char *name, UINT n)
{
    static const float zero[4];
    D3D11_VIEWPORT vp = {0.0f, 0.0f, BIAS_SIZE, BIAS_SIZE, 0.0f, 1.0f};
    D3D11_BUFFER_DESC buffer_desc = {0};
    D3D11_SUBRESOURCE_DATA data = {0};
    D3D11_RASTERIZER_DESC rs_desc = {0};
    ID3D11Texture2D *color = NULL, *staging = NULL;
    ID3D11Buffer *ib16 = NULL, *ib32 = NULL;
    ID3D11RasterizerState *rs = NULL;
    ID3D11RenderTargetView *rtv = NULL;
    D3D11_MAPPED_SUBRESOURCE map;
    USHORT indices16[ARRAYSIZE(index16_units) * 4];
    UINT indices32[ARRAYSIZE(index32_units) * 4];
    UINT i, j, u, lit;
    int ok = 1, ret = 3;
    HRESULT hr = E_INVALIDARG;

    if (n > 4)
        goto done;
    for (i = 0; i < ARRAYSIZE(index16_units); i++)
        for (j = 0; j < n; j++)
            indices16[i * n + j] = index16_units[i] * n + j;
    for (i = 0; i < ARRAYSIZE(index32_units); i++)
        for (j = 0; j < n; j++)
            indices32[i * n + j] = index32_units[i] * n + j;
    buffer_desc.Usage = D3D11_USAGE_DEFAULT;
    buffer_desc.BindFlags = D3D11_BIND_INDEX_BUFFER;
    buffer_desc.ByteWidth = ARRAYSIZE(index16_units) * n * sizeof(*indices16);
    data.pSysMem = indices16;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &ib16)))
        goto done;
    buffer_desc.ByteWidth = ARRAYSIZE(index32_units) * n * sizeof(*indices32);
    data.pSysMem = indices32;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &ib32)))
        goto done;
    rs_desc.FillMode = D3D11_FILL_SOLID;
    rs_desc.CullMode = D3D11_CULL_NONE;
    rs_desc.DepthClipEnable = TRUE;
    if (FAILED(hr = ID3D11Device_CreateRasterizerState(device, &rs_desc, &rs))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_BIND_RENDER_TARGET, FALSE,
                                           &color))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_R8G8B8A8_UNORM, 0, TRUE, &staging))
        || FAILED(hr = ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)color, NULL, &rtv)))
        goto done;

    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &rtv, NULL);
    ID3D11DeviceContext_RSSetViewports(context, 1, &vp);
    ID3D11DeviceContext_RSSetState(context, rs);
    for (i = 0; i < ARRAYSIZE(index_passes); i++)
    {
        /* The clear before and the copy after end the render pass, so the
           first draw of each pass builds its own pipeline. */
        ID3D11DeviceContext_ClearRenderTargetView(context, rtv, zero);
        for (j = 0; j < ARRAYSIZE(index_passes[i].draws); j++)
        {
            const struct index_draw *draw = &index_passes[i].draws[j];

            if (draw->format == DXGI_FORMAT_UNKNOWN)
            {
                ID3D11DeviceContext_Draw(context, n, draw->start * n);
                continue;
            }
            ID3D11DeviceContext_IASetIndexBuffer(context, draw->format == DXGI_FORMAT_R16_UINT ? ib16 : ib32,
                                                 draw->format, 0);
            ID3D11DeviceContext_DrawIndexed(context, n, draw->start * n, 0);
        }
        ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging, (ID3D11Resource *)color);
        if (FAILED(hr = ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &map)))
            goto done;
        lit = 0;
        for (u = 0; u < INDEX_UNITS; u++)
        {
            UINT pixel = ((const UINT *)((const BYTE *)map.pData + BIAS_SIZE / 2 * map.RowPitch))[INDEX_UNIT_PIXEL(u)];

            /* The pixel shader's red with any alpha */
            if ((pixel & 0x00ffffff) == 0x0000ff)
                lit |= 1u << u;
        }
        ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, 0);
        printf(PREFIX "%s: %s: units lit %#x, expected %#x %s\n", name, index_passes[i].what, lit,
               index_passes[i].lit, lit == index_passes[i].lit ? "ok" : "FAILED");
        ok &= lit == index_passes[i].lit;
    }
    printf(PREFIX "%s check %s\n", name, ok ? "ok" : "FAILED");
    ret = ok ? 0 : 1;

done:
    if (ret == 3)
        printf(PREFIX "%s: setup or readback failed, %#lx\n", name, (unsigned long)hr);
    ID3D11DeviceContext_RSSetState(context, NULL);
    if (rtv)
        ID3D11RenderTargetView_Release(rtv);
    if (staging)
        ID3D11Texture2D_Release(staging);
    if (color)
        ID3D11Texture2D_Release(color);
    if (rs)
        ID3D11RasterizerState_Release(rs);
    if (ib32)
        ID3D11Buffer_Release(ib32);
    if (ib16)
        ID3D11Buffer_Release(ib16);
    return ret;
}

static int check_geometry_index(ID3D11Device *device, ID3D11DeviceContext *context)
{
    /* One point per unit for the geometry shader; vs_code reads POSITION,
       COLOR and COLOR1. */
    float points[INDEX_UNITS][12] = {{0}};
    D3D11_BUFFER_DESC buffer_desc = {0};
    D3D11_SUBRESOURCE_DATA data = {0};
    ID3D11InputLayout *layout = NULL;
    ID3D11GeometryShader *gs = NULL;
    ID3D11VertexShader *vs = NULL;
    ID3D11PixelShader *ps = NULL;
    ID3D11Buffer *vb = NULL;
    UINT stride = sizeof(points[0]), offset = 0, u;
    int ret = 3;
    HRESULT hr;

    for (u = 0; u < INDEX_UNITS; u++)
    {
        points[u][0] = INDEX_UNIT_X(u);
        points[u][2] = 0.5f;
        points[u][3] = 1.0f;
    }
    buffer_desc.Usage = D3D11_USAGE_DEFAULT;
    buffer_desc.BindFlags = D3D11_BIND_VERTEX_BUFFER;
    buffer_desc.ByteWidth = sizeof(points);
    data.pSysMem = points;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &vb))
        || FAILED(hr = ID3D11Device_CreateVertexShader(device, vs_code, sizeof(vs_code), NULL, &vs))
        || FAILED(hr = ID3D11Device_CreateGeometryShader(device, point_quad_gs_code, sizeof(point_quad_gs_code),
                                                         NULL, &gs))
        || FAILED(hr = ID3D11Device_CreatePixelShader(device, red_ps_code, sizeof(red_ps_code), NULL, &ps))
        || FAILED(hr = ID3D11Device_CreateInputLayout(device, layout_desc, 3, vs_code, sizeof(vs_code), &layout)))
    {
        printf(PREFIX "geometry-index: setup failed, %#lx\n", (unsigned long)hr);
        goto done;
    }

    ID3D11DeviceContext_IASetInputLayout(context, layout);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &stride, &offset);
    ID3D11DeviceContext_VSSetShader(context, vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, gs, NULL, 0);
    ID3D11DeviceContext_PSSetShader(context, ps, NULL, 0);
    ret = check_index_draws(device, context, "geometry-index", 1);
    ID3D11DeviceContext_GSSetShader(context, NULL, NULL, 0);

done:
    if (layout)
        ID3D11InputLayout_Release(layout);
    if (ps)
        ID3D11PixelShader_Release(ps);
    if (gs)
        ID3D11GeometryShader_Release(gs);
    if (vs)
        ID3D11VertexShader_Release(vs);
    if (vb)
        ID3D11Buffer_Release(vb);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return ret;
}

static int check_tessellation_index(ID3D11Device *device, ID3D11DeviceContext *context)
{
    /* Tessellation factors of 1: each patch becomes the two triangles of
       its quad. */
    static const float factors[8] = {1.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f};
    /* One patch per unit, a quad 0.4 wide: control points 0 and 1 along
       the bottom, 2 and 3 along the top. */
    float patches[INDEX_UNITS][4][3];
    D3D11_BUFFER_DESC buffer_desc = {0};
    D3D11_SUBRESOURCE_DATA data = {0};
    ID3D11InputLayout *layout = NULL;
    ID3D11Buffer *vb = NULL, *cb = NULL;
    ID3D11VertexShader *vs = NULL;
    ID3D11HullShader *hs = NULL;
    ID3D11DomainShader *ds = NULL;
    ID3D11PixelShader *ps = NULL;
    UINT stride = sizeof(patches[0][0]), offset = 0, u, k;
    int ret = 3;
    HRESULT hr;

    for (u = 0; u < INDEX_UNITS; u++)
    {
        for (k = 0; k < 4; k++)
        {
            patches[u][k][0] = INDEX_UNIT_X(u) + (k & 1 ? 0.2f : -0.2f);
            patches[u][k][1] = k & 2 ? 0.2f : -0.2f;
            patches[u][k][2] = 0.5f;
        }
    }
    buffer_desc.Usage = D3D11_USAGE_DEFAULT;
    buffer_desc.BindFlags = D3D11_BIND_VERTEX_BUFFER;
    buffer_desc.ByteWidth = sizeof(patches);
    data.pSysMem = patches;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &vb)))
        goto failed;
    buffer_desc.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
    buffer_desc.ByteWidth = sizeof(factors);
    data.pSysMem = factors;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &cb))
        || FAILED(hr = ID3D11Device_CreateVertexShader(device, quad_vs_code, sizeof(quad_vs_code), NULL, &vs))
        || FAILED(hr = ID3D11Device_CreateHullShader(device, quad_hs_code, sizeof(quad_hs_code), NULL, &hs))
        || FAILED(hr = ID3D11Device_CreateDomainShader(device, quad_ds_code, sizeof(quad_ds_code), NULL, &ds))
        || FAILED(hr = ID3D11Device_CreatePixelShader(device, red_ps_code, sizeof(red_ps_code), NULL, &ps))
        || FAILED(hr = ID3D11Device_CreateInputLayout(device, quad_layout_desc, 1, quad_vs_code,
                                                      sizeof(quad_vs_code), &layout)))
        goto failed;

    ID3D11DeviceContext_IASetInputLayout(context, layout);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_4_CONTROL_POINT_PATCHLIST);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &stride, &offset);
    ID3D11DeviceContext_VSSetShader(context, vs, NULL, 0);
    ID3D11DeviceContext_HSSetShader(context, hs, NULL, 0);
    ID3D11DeviceContext_HSSetConstantBuffers(context, 0, 1, &cb);
    ID3D11DeviceContext_DSSetShader(context, ds, NULL, 0);
    ID3D11DeviceContext_PSSetShader(context, ps, NULL, 0);
    ret = check_index_draws(device, context, "tessellation-index", 4);
    ID3D11DeviceContext_HSSetShader(context, NULL, NULL, 0);
    ID3D11DeviceContext_DSSetShader(context, NULL, NULL, 0);
    goto done;

failed:
    printf(PREFIX "tessellation-index: setup failed, %#lx\n", (unsigned long)hr);
done:
    if (layout)
        ID3D11InputLayout_Release(layout);
    if (ps)
        ID3D11PixelShader_Release(ps);
    if (ds)
        ID3D11DomainShader_Release(ds);
    if (hs)
        ID3D11HullShader_Release(hs);
    if (vs)
        ID3D11VertexShader_Release(vs);
    if (cb)
        ID3D11Buffer_Release(cb);
    if (vb)
        ID3D11Buffer_Release(vb);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return ret;
}

static int check_release_after_draw(ID3D11Device *device, ID3D11DeviceContext *context)
{
    /* stream output of COLOR, for the second geometry shader */
    static const D3D11_SO_DECLARATION_ENTRY so_color_decl[] = {
        {0, "COLOR", 0, 0, 4, 0},
    };
    /* Five points for vs_code, which reads POSITION, COLOR and COLOR1. */
    static const float points[5][12] = {
        {-0.5f, -0.5f, 0.5f, 1.0f, 1.0f, 0.0f, 0.0f, 1.0f},
        {0.5f, -0.5f, 0.5f, 1.0f, 0.0f, 1.0f, 0.0f, 1.0f},
        {-0.5f, 0.5f, 0.5f, 1.0f, 0.0f, 0.0f, 1.0f, 1.0f},
        {0.5f, 0.5f, 0.5f, 1.0f, 1.0f, 1.0f, 0.0f, 1.0f},
        {0.0f, 0.0f, 0.5f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f},
    };
    D3D11_VIEWPORT vp = {0.0f, 0.0f, BIAS_SIZE, BIAS_SIZE, 0.0f, 1.0f};
    D3D11_BUFFER_DESC buffer_desc = {0};
    D3D11_SUBRESOURCE_DATA data = {0};
    ID3D11GeometryShader *gs_position = NULL, *gs_color = NULL;
    ID3D11Buffer *vb = NULL, *so = NULL;
    ID3D11InputLayout *layout = NULL;
    ID3D11RenderTargetView *rtv = NULL;
    ID3D11VertexShader *vs = NULL;
    ID3D11Texture2D *color = NULL;
    UINT stride = sizeof(points[0]), so_stride = 16, offset = 0;
    struct tracker tracker;
    HRESULT hr;

    hr = ID3D11Device_SetPrivateDataInterface(device, &tracker_key, tracker_init(&tracker));
    if (!tracker_hand_over(&tracker, hr))
    {
        printf(PREFIX "release-after-draw: SetPrivateDataInterface %#lx, tracker references %ld\n",
               (unsigned long)hr, tracker.refcount);
        return 3;
    }
    buffer_desc.Usage = D3D11_USAGE_DEFAULT;
    buffer_desc.BindFlags = D3D11_BIND_VERTEX_BUFFER;
    buffer_desc.ByteWidth = sizeof(points);
    data.pSysMem = points;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &vb)))
        goto failed;
    buffer_desc.BindFlags = D3D11_BIND_STREAM_OUTPUT;
    buffer_desc.ByteWidth = 64 * so_stride;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, NULL, &so))
        || FAILED(hr = ID3D11Device_CreateVertexShader(device, vs_code, sizeof(vs_code), NULL, &vs))
        || FAILED(hr = ID3D11Device_CreateInputLayout(device, layout_desc, 3, vs_code, sizeof(vs_code), &layout))
        || FAILED(hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(
                      device, vs_code, sizeof(vs_code), so_decl, 1, &so_stride, 1, D3D11_SO_NO_RASTERIZED_STREAM,
                      NULL, &gs_position))
        || FAILED(hr = ID3D11Device_CreateGeometryShaderWithStreamOutput(
                      device, vs_code, sizeof(vs_code), so_color_decl, 1, &so_stride, 1,
                      D3D11_SO_NO_RASTERIZED_STREAM, NULL, &gs_color))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_BIND_RENDER_TARGET, FALSE,
                                           &color))
        || FAILED(hr = ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)color, NULL, &rtv)))
        goto failed;

    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &rtv, NULL);
    ID3D11DeviceContext_RSSetViewports(context, 1, &vp);
    ID3D11DeviceContext_IASetInputLayout(context, layout);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &vb, &stride, &offset);
    ID3D11DeviceContext_SOSetTargets(context, 1, &so, &offset);
    ID3D11DeviceContext_VSSetShader(context, vs, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, gs_position, NULL, 0);

    /* The draw and the release, in the order of the program the crash was
       found with. */
    ID3D11DeviceContext_GSSetShader(context, NULL, NULL, 0);
    ID3D11DeviceContext_VSSetShader(context, vs, NULL, 0);
    ID3D11DeviceContext_IASetInputLayout(context, layout);
    ID3D11DeviceContext_Draw(context, 5, 0);
    ID3D11DeviceContext_Flush(context);
    ID3D11DeviceContext_ClearState(context);

    ID3D11RenderTargetView_Release(rtv);
    ID3D11Texture2D_Release(color);
    ID3D11GeometryShader_Release(gs_color);
    ID3D11GeometryShader_Release(gs_position);
    ID3D11InputLayout_Release(layout);
    ID3D11VertexShader_Release(vs);
    ID3D11Buffer_Release(so);
    ID3D11Buffer_Release(vb);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);

    printf(PREFIX "release-after-draw: device %s by its release\n", tracker.destroyed ? "destroyed" : "kept");
    if (!tracker.destroyed)
    {
        fail_now("release-after-draw");
        return 1;
    }
    printf(PREFIX "release-after-draw check ok\n");
    return 0;

failed:
    printf(PREFIX "release-after-draw: setup failed, %#lx\n", (unsigned long)hr);
    if (rtv)
        ID3D11RenderTargetView_Release(rtv);
    if (color)
        ID3D11Texture2D_Release(color);
    if (gs_color)
        ID3D11GeometryShader_Release(gs_color);
    if (gs_position)
        ID3D11GeometryShader_Release(gs_position);
    if (layout)
        ID3D11InputLayout_Release(layout);
    if (vs)
        ID3D11VertexShader_Release(vs);
    if (so)
        ID3D11Buffer_Release(so);
    if (vb)
        ID3D11Buffer_Release(vb);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return 3;
}

#define SHADER_WORDS 256

struct chunk
{
    const DWORD *words;
    UINT count;
};

#define CHUNK(a) {a, ARRAYSIZE(a)}

/* build_container: the DXBC container of the count chunks in out, of
   SHADER_WORDS words, with a zero checksum, which DXMT does not check. The
   last chunk is the shader chunk, whose size and length are set from its
   word count. Returns the container's size, 0 when it does not fit. */
static UINT build_container(DWORD *out, const struct chunk *chunks, UINT count)
{
    UINT size = 32 + 4 * count, i;
    DWORD *shader;

    if (count > 4)
        return 0;
    memset(out, 0, 32);
    memcpy(out, "DXBC", 4);
    out[5] = 1;
    out[7] = count;
    for (i = 0; i < count; i++)
    {
        if (size + 4 * chunks[i].count > SHADER_WORDS * sizeof(*out))
            return 0;
        out[8 + i] = size;
        memcpy((BYTE *)out + size, chunks[i].words, 4 * chunks[i].count);
        size += 4 * chunks[i].count;
    }
    shader = (DWORD *)((BYTE *)out + out[8 + count - 1]);
    shader[1] = 4 * (chunks[count - 1].count - 2);
    shader[3] = chunks[count - 1].count - 2;
    out[6] = size;
    return size;
}

/* build_shader: build_container of an input signature, an output signature
   and a shader chunk */
static UINT build_shader(DWORD *out, struct chunk isgn, struct chunk osgn, struct chunk shdr)
{
    const struct chunk chunks[] = {isgn, osgn, shdr};

    return build_container(out, chunks, ARRAYSIZE(chunks));
}

static HRESULT create_gs(ID3D11Device *device, struct chunk isgn, struct chunk osgn, struct chunk shdr,
                         ID3D11GeometryShader **gs)
{
    DWORD code[SHADER_WORDS];
    UINT size = build_shader(code, isgn, osgn, shdr);

    return size ? ID3D11Device_CreateGeometryShader(device, code, size, NULL, gs) : E_INVALIDARG;
}

/* draw_pixel: clear the target to 0, draw count vertices and return the
   pixel at x, y in *pixel, or FALSE when the readback fails */
static BOOL draw_pixel(ID3D11DeviceContext *context, ID3D11RenderTargetView *rtv, ID3D11Texture2D *color,
                       ID3D11Texture2D *staging, UINT count, UINT x, UINT y, UINT *pixel)
{
    static const float zero[4];
    D3D11_MAPPED_SUBRESOURCE map;

    ID3D11DeviceContext_ClearRenderTargetView(context, rtv, zero);
    ID3D11DeviceContext_Draw(context, count, 0);
    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging, (ID3D11Resource *)color);
    if (FAILED(ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &map)))
        return FALSE;
    *pixel = ((const UINT *)((const BYTE *)map.pData + y * map.RowPitch))[x];
    ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, 0);
    return TRUE;
}

/* draw_slices: clear both slices of the target to 0, draw count vertices
   and return the pixel at x, y of slice 0 and of slice 1, or FALSE when the
   readback fails */
static BOOL draw_slices(ID3D11DeviceContext *context, ID3D11RenderTargetView *rtv, ID3D11Texture2D *color,
                        ID3D11Texture2D *staging, UINT count, UINT x, UINT y, UINT *pixels)
{
    static const float zero[4];
    D3D11_MAPPED_SUBRESOURCE map;
    UINT slice;

    ID3D11DeviceContext_ClearRenderTargetView(context, rtv, zero);
    ID3D11DeviceContext_Draw(context, count, 0);
    ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging, (ID3D11Resource *)color);
    for (slice = 0; slice < 2; slice++)
    {
        if (FAILED(ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, slice, D3D11_MAP_READ, 0, &map)))
            return FALSE;
        pixels[slice] = ((const UINT *)((const BYTE *)map.pData + y * map.RowPitch))[x];
        ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, slice);
    }
    return TRUE;
}

static HRESULT create_slices(ID3D11Device *device, ID3D11Texture2D **color, ID3D11Texture2D **staging,
                             ID3D11RenderTargetView **rtv)
{
    D3D11_TEXTURE2D_DESC desc = {0};
    HRESULT hr;

    desc.Width = BIAS_SIZE;
    desc.Height = BIAS_SIZE;
    desc.MipLevels = 1;
    desc.ArraySize = 2;
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET;
    if (FAILED(hr = ID3D11Device_CreateTexture2D(device, &desc, NULL, color)))
        return hr;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    if (FAILED(hr = ID3D11Device_CreateTexture2D(device, &desc, NULL, staging)))
        return hr;
    return ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)*color, NULL, rtv);
}

/* Red with any alpha, as red_ps_code and color_vs_shdr give it */
#define IS_RED(pixel) (((pixel) & 0x00ffffff) == 0x0000ff)

enum pass_through_result
{
    /* the vertex shader's red, as a pass-through draws it */
    DRAWS_RED,
    /* nothing at the pixel read back */
    DRAWS_NOTHING,
    /* the position's color: no red, about 0.5 green */
    DRAWS_POSITION,
};

static int check_pass_through(ID3D11Device *device, ID3D11DeviceContext *context)
{
    static const struct
    {
        const char *what;
        struct chunk isgn, shdr;
        /* through clip_vs_shdr instead of color_vs_shdr */
        BOOL clip;
        enum pass_through_result result;
    } variants[] = {
        {"passes its triangle through", CHUNK(color_in_isgn), CHUNK(pass_gs_shdr), FALSE, DRAWS_RED},
        {"emits the vertices in another order", CHUNK(color_in_isgn), CHUNK(order_gs_shdr), FALSE, DRAWS_NOTHING},
        {"writes the outputs for its first vertex only", CHUNK(color_in_isgn), CHUNK(first_gs_shdr), FALSE,
         DRAWS_NOTHING},
        {"emits two vertices", CHUNK(color_in_isgn), CHUNK(part_gs_shdr), FALSE, DRAWS_NOTHING},
        {"emits points", CHUNK(color_in_isgn), CHUNK(points_gs_shdr), FALSE, DRAWS_NOTHING},
        {"moves a constant to the position", CHUNK(color_in_isgn), CHUNK(constant_gs_shdr), FALSE, DRAWS_NOTHING},
        {"swaps x and y", CHUNK(color_in_isgn), CHUNK(swizzle_gs_shdr), FALSE, DRAWS_NOTHING},
        {"negates x", CHUNK(color_in_isgn), CHUNK(negate_gs_shdr), FALSE, DRAWS_NOTHING},
        {"saturates the position", CHUNK(color_in_isgn), CHUNK(saturate_gs_shdr), FALSE, DRAWS_NOTHING},
        {"moves the position to COLOR", CHUNK(color_in_isgn), CHUNK(register_gs_shdr), FALSE, DRAWS_POSITION},
        /* The vertex shader's clip distance of -1 clips everything; D3D clips
           by the geometry shader's output, which has none */
        {"drops the vertex shader's SV_ClipDistance", CHUNK(clip_in_isgn), CHUNK(pass_gs_shdr), TRUE, DRAWS_RED},
    };
    /* Tessellation factors of 1: the patch becomes the two triangles of its
       quad. */
    static const float factors[8] = {1.0f, 1.0f, 1.0f, 1.0f, 1.0f, 1.0f};
    /* A triangle strip over the left half of the target; the patch of four
       control points of a quad around the centre, 0 and 1 along the
       bottom, 2 and 3 along the top. */
    static const float half_quad[] = {
        -1.0f, -1.0f, 0.5f, -1.0f, 1.0f, 0.5f, 0.0f, -1.0f, 0.5f, 0.0f, 1.0f, 0.5f,
    };
    static const float patch[] = {
        -0.5f, -0.5f, 0.5f, 0.5f, -0.5f, 0.5f, -0.5f, 0.5f, 0.5f, 0.5f, 0.5f, 0.5f,
    };
    D3D11_VIEWPORT vp = {0.0f, 0.0f, BIAS_SIZE, BIAS_SIZE, 0.0f, 1.0f};
    D3D11_BUFFER_DESC buffer_desc = {0};
    D3D11_SUBRESOURCE_DATA data = {0};
    D3D11_RASTERIZER_DESC rs_desc = {0};
    ID3D11GeometryShader *gs[ARRAYSIZE(variants)] = {0}, *tess_gs = NULL, *layer_gs = NULL;
    ID3D11Buffer *quad_vb = NULL, *patch_vb = NULL, *cb = NULL;
    ID3D11Texture2D *color = NULL, *staging = NULL, *slices = NULL, *slices_staging = NULL;
    ID3D11VertexShader *color_vs = NULL, *clip_vs = NULL, *vs = NULL;
    ID3D11PixelShader *color_ps = NULL, *ps = NULL;
    ID3D11RasterizerState *no_cull = NULL;
    ID3D11InputLayout *layout = NULL;
    ID3D11RenderTargetView *rtv = NULL, *slices_rtv = NULL;
    ID3D11HullShader *hs = NULL;
    ID3D11DomainShader *ds = NULL, *layer_ds = NULL;
    UINT stride = 3 * sizeof(float), offset = 0, pixel, pixels[2], size, i;
    const struct chunk layer_ds_chunks[] = {
        {QUAD_DS_ISGN, QUAD_DS_ISGN_WORDS}, {QUAD_DS_PCSG, QUAD_DS_PCSG_WORDS}, CHUNK(layer_out_osgn),
        CHUNK(layer_ds_shex),
    };
    DWORD code[SHADER_WORDS];
    int ok = 1, ret = 3;
    HRESULT hr;

    buffer_desc.Usage = D3D11_USAGE_DEFAULT;
    buffer_desc.BindFlags = D3D11_BIND_VERTEX_BUFFER;
    buffer_desc.ByteWidth = sizeof(half_quad);
    data.pSysMem = half_quad;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &quad_vb)))
        goto done;
    buffer_desc.ByteWidth = sizeof(patch);
    data.pSysMem = patch;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &patch_vb)))
        goto done;
    buffer_desc.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
    buffer_desc.ByteWidth = sizeof(factors);
    data.pSysMem = factors;
    if (FAILED(hr = ID3D11Device_CreateBuffer(device, &buffer_desc, &data, &cb)))
        goto done;
    hr = E_INVALIDARG;
    if (!(size = build_shader(code, (struct chunk)CHUNK(position_vs_isgn), (struct chunk)CHUNK(color_out_osgn),
                              (struct chunk)CHUNK(color_vs_shdr)))
        || FAILED(hr = ID3D11Device_CreateVertexShader(device, code, size, NULL, &color_vs)))
        goto done;
    hr = E_INVALIDARG;
    if (!(size = build_shader(code, (struct chunk)CHUNK(position_vs_isgn), (struct chunk)CHUNK(clip_out_osgn),
                              (struct chunk)CHUNK(clip_vs_shdr)))
        || FAILED(hr = ID3D11Device_CreateVertexShader(device, code, size, NULL, &clip_vs)))
        goto done;
    hr = E_INVALIDARG;
    if (!(size = build_container(code, layer_ds_chunks, ARRAYSIZE(layer_ds_chunks)))
        || FAILED(hr = ID3D11Device_CreateDomainShader(device, code, size, NULL, &layer_ds)))
        goto done;
    hr = E_INVALIDARG;
    if (!(size = build_shader(code, (struct chunk)CHUNK(color_ps_isgn), (struct chunk)CHUNK(target_ps_osgn),
                              (struct chunk)CHUNK(color_ps_shdr)))
        || FAILED(hr = ID3D11Device_CreatePixelShader(device, code, size, NULL, &color_ps)))
        goto done;
    for (i = 0; i < ARRAYSIZE(variants); i++)
        if (FAILED(hr = create_gs(device, variants[i].isgn, (struct chunk)CHUNK(color_out_osgn), variants[i].shdr,
                                  &gs[i])))
            goto done;
    rs_desc.FillMode = D3D11_FILL_SOLID;
    rs_desc.CullMode = D3D11_CULL_NONE;
    rs_desc.DepthClipEnable = TRUE;
    if (FAILED(hr = create_gs(device, (struct chunk)CHUNK(position_in_isgn), (struct chunk)CHUNK(position_out_osgn),
                              (struct chunk)CHUNK(position_pass_gs_shdr), &tess_gs))
        || FAILED(hr = create_gs(device, (struct chunk)CHUNK(layer_gs_isgn), (struct chunk)CHUNK(layer_gs_osgn),
                                 (struct chunk)CHUNK(layer_pass_gs_shdr), &layer_gs))
        || FAILED(hr = create_slices(device, &slices, &slices_staging, &slices_rtv))
        || FAILED(hr = ID3D11Device_CreateVertexShader(device, quad_vs_code, sizeof(quad_vs_code), NULL, &vs))
        || FAILED(hr = ID3D11Device_CreateHullShader(device, quad_hs_code, sizeof(quad_hs_code), NULL, &hs))
        || FAILED(hr = ID3D11Device_CreateDomainShader(device, quad_ds_code, sizeof(quad_ds_code), NULL, &ds))
        || FAILED(hr = ID3D11Device_CreatePixelShader(device, red_ps_code, sizeof(red_ps_code), NULL, &ps))
        || FAILED(hr = ID3D11Device_CreateInputLayout(device, quad_layout_desc, 1, quad_vs_code,
                                                      sizeof(quad_vs_code), &layout))
        || FAILED(hr = ID3D11Device_CreateRasterizerState(device, &rs_desc, &no_cull))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_BIND_RENDER_TARGET, FALSE,
                                           &color))
        || FAILED(hr = create_bias_texture(device, DXGI_FORMAT_R8G8B8A8_UNORM, 0, TRUE, &staging))
        || FAILED(hr = ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)color, NULL, &rtv)))
        goto done;

    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &rtv, NULL);
    ID3D11DeviceContext_RSSetViewports(context, 1, &vp);
    ID3D11DeviceContext_IASetInputLayout(context, layout);
    hr = E_FAIL;

    /* Tessellation with and without the pass-through geometry shader, which
       DXMT draws only when it takes the shader as a pass-through. */
    ID3D11DeviceContext_RSSetState(context, no_cull);
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_4_CONTROL_POINT_PATCHLIST);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &patch_vb, &stride, &offset);
    ID3D11DeviceContext_VSSetShader(context, vs, NULL, 0);
    ID3D11DeviceContext_HSSetShader(context, hs, NULL, 0);
    ID3D11DeviceContext_HSSetConstantBuffers(context, 0, 1, &cb);
    ID3D11DeviceContext_DSSetShader(context, ds, NULL, 0);
    ID3D11DeviceContext_PSSetShader(context, ps, NULL, 0);
    if (!draw_pixel(context, rtv, color, staging, 4, BIAS_SIZE / 2, BIAS_SIZE / 2, &pixel))
        goto done;
    printf(PREFIX "pass-through: tessellation without a geometry shader: pixel %#x %s\n", pixel,
           IS_RED(pixel) ? "ok" : "FAILED");
    ok &= IS_RED(pixel);
    ID3D11DeviceContext_GSSetShader(context, tess_gs, NULL, 0);
    if (!draw_pixel(context, rtv, color, staging, 4, BIAS_SIZE / 2, BIAS_SIZE / 2, &pixel))
        goto done;
    printf(PREFIX "pass-through: tessellation through a geometry shader that passes its triangles through: "
                  "pixel %#x %s\n", pixel, IS_RED(pixel) ? "ok" : "FAILED");
    ok &= IS_RED(pixel);
    /* The same through layer_pass_gs, with the domain shader that writes a
       LAYER of 1, into a target of two slices: only slice 1 is drawn. */
    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &slices_rtv, NULL);
    ID3D11DeviceContext_DSSetShader(context, layer_ds, NULL, 0);
    ID3D11DeviceContext_GSSetShader(context, layer_gs, NULL, 0);
    if (!draw_slices(context, slices_rtv, slices, slices_staging, 4, BIAS_SIZE / 2, BIAS_SIZE / 2, pixels))
        goto done;
    printf(PREFIX "pass-through: tessellation through layer_pass_gs, LAYER 1: slice 0 %#x, slice 1 %#x %s\n",
           pixels[0], pixels[1], !pixels[0] && IS_RED(pixels[1]) ? "ok" : "FAILED");
    ok &= !pixels[0] && IS_RED(pixels[1]);
    ID3D11DeviceContext_OMSetRenderTargets(context, 1, &rtv, NULL);
    ID3D11DeviceContext_HSSetShader(context, NULL, NULL, 0);
    ID3D11DeviceContext_DSSetShader(context, NULL, NULL, 0);
    ID3D11DeviceContext_RSSetState(context, NULL);

    /* The half quad through each geometry shader with the default rasterizer
       state, which culls the triangles turned away, read back at the centre
       of the left half's top half. */
    ID3D11DeviceContext_IASetPrimitiveTopology(context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
    ID3D11DeviceContext_IASetVertexBuffers(context, 0, 1, &quad_vb, &stride, &offset);
    ID3D11DeviceContext_PSSetShader(context, color_ps, NULL, 0);
    for (i = 0; i < ARRAYSIZE(variants); i++)
    {
        int right;

        ID3D11DeviceContext_VSSetShader(context, variants[i].clip ? clip_vs : color_vs, NULL, 0);
        ID3D11DeviceContext_GSSetShader(context, gs[i], NULL, 0);
        if (!draw_pixel(context, rtv, color, staging, 4, BIAS_SIZE / 4, BIAS_SIZE / 4, &pixel))
            goto done;
        if (variants[i].result == DRAWS_RED)
            right = IS_RED(pixel);
        else if (variants[i].result == DRAWS_POSITION)
            right = !(pixel & 0xff) && ((pixel >> 8) & 0xff) >= 0x40;
        else
            right = !(pixel & 0x00ffffff);
        printf(PREFIX "pass-through: a geometry shader that %s: pixel %#x %s\n", variants[i].what, pixel,
               right ? "ok" : "FAILED");
        ok &= right;
    }
    ID3D11DeviceContext_GSSetShader(context, NULL, NULL, 0);

    printf(PREFIX "pass-through check %s\n", ok ? "ok" : "FAILED");
    ret = ok ? 0 : 1;

done:
    if (ret == 3)
        printf(PREFIX "pass-through: setup or readback failed, %#lx\n", (unsigned long)hr);
    ID3D11DeviceContext_ClearState(context);
    if (slices_rtv)
        ID3D11RenderTargetView_Release(slices_rtv);
    if (slices_staging)
        ID3D11Texture2D_Release(slices_staging);
    if (slices)
        ID3D11Texture2D_Release(slices);
    if (rtv)
        ID3D11RenderTargetView_Release(rtv);
    if (staging)
        ID3D11Texture2D_Release(staging);
    if (color)
        ID3D11Texture2D_Release(color);
    if (no_cull)
        ID3D11RasterizerState_Release(no_cull);
    if (layout)
        ID3D11InputLayout_Release(layout);
    if (ps)
        ID3D11PixelShader_Release(ps);
    if (layer_ds)
        ID3D11DomainShader_Release(layer_ds);
    if (ds)
        ID3D11DomainShader_Release(ds);
    if (hs)
        ID3D11HullShader_Release(hs);
    if (vs)
        ID3D11VertexShader_Release(vs);
    if (layer_gs)
        ID3D11GeometryShader_Release(layer_gs);
    if (tess_gs)
        ID3D11GeometryShader_Release(tess_gs);
    for (i = 0; i < ARRAYSIZE(gs); i++)
        if (gs[i])
            ID3D11GeometryShader_Release(gs[i]);
    if (color_ps)
        ID3D11PixelShader_Release(color_ps);
    if (clip_vs)
        ID3D11VertexShader_Release(clip_vs);
    if (color_vs)
        ID3D11VertexShader_Release(color_vs);
    if (cb)
        ID3D11Buffer_Release(cb);
    if (patch_vb)
        ID3D11Buffer_Release(patch_vb);
    if (quad_vb)
        ID3D11Buffer_Release(quad_vb);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return ret;
}

static int check_swap_chain_detach(ID3D11Device *device, ID3D11DeviceContext *context)
{
    DXGI_SWAP_CHAIN_DESC desc = {0};
    IDXGISwapChain *swap_chain = NULL;
    IDXGIDevice *dxgi_device = NULL;
    IDXGIAdapter *adapter = NULL;
    IDXGIFactory *factory = NULL;
    void (*hold)(IUnknown *) = NULL;
    HMODULE module;
    HWND window;
    HRESULT hr = E_FAIL;
    int ret = 3;

    if ((module = LoadLibraryA("dxmthold-" ARCH ".dll")))
        hold = (void (*)(IUnknown *))GetProcAddress(module, "hold");
    window = CreateWindowA("static", "dxmt swap-chain-detach", WS_POPUP, 0, 0, BIAS_SIZE, BIAS_SIZE, NULL, NULL,
                           NULL, NULL);
    if (!hold || !window)
    {
        printf(PREFIX "swap-chain-detach: dxmthold-" ARCH ".dll %s, window %p\n", hold ? "loaded" : "not loaded",
               (void *)window);
        goto done;
    }
    desc.BufferDesc.Width = BIAS_SIZE;
    desc.BufferDesc.Height = BIAS_SIZE;
    desc.BufferDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    desc.BufferCount = 1;
    desc.OutputWindow = window;
    desc.Windowed = TRUE;
    desc.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;
    if (FAILED(hr = ID3D11Device_QueryInterface(device, &IID_IDXGIDevice, (void **)&dxgi_device))
        || FAILED(hr = IDXGIDevice_GetAdapter(dxgi_device, &adapter))
        || FAILED(hr = IDXGIAdapter_GetParent(adapter, &IID_IDXGIFactory, (void **)&factory))
        || FAILED(hr = IDXGIFactory_CreateSwapChain(factory, (IUnknown *)device, &desc, &swap_chain))
        || FAILED(hr = IDXGISwapChain_Present(swap_chain, 0, 0)))
    {
        printf(PREFIX "swap-chain-detach: setup failed, %#lx\n", (unsigned long)hr);
        goto done;
    }
    /* The DLL's reference is then the swap chain's only one, and the DLL
       releases it in its DLL_PROCESS_DETACH at process exit. */
    hold((IUnknown *)swap_chain);
    printf(PREFIX "swap-chain-detach: the swap chain is left to the DLL detach at process exit\n");
    fflush(stdout);
    ret = 0;

done:
    if (swap_chain)
        IDXGISwapChain_Release(swap_chain);
    if (factory)
        IDXGIFactory_Release(factory);
    if (adapter)
        IDXGIAdapter_Release(adapter);
    if (dxgi_device)
        IDXGIDevice_Release(dxgi_device);
    ID3D11DeviceContext_Release(context);
    ID3D11Device_Release(device);
    return ret;
}

static const struct
{
    const char *name;
    int (*run)(ID3D11Device *device, ID3D11DeviceContext *context);
} cases[] = {
    {"bytecode", check_bytecode},
    {"swap-state", check_swap_state},
    {"deferred", check_deferred},
    {"depth-bias", check_depth_bias},
    {"geometry-index", check_geometry_index},
    {"tessellation-index", check_tessellation_index},
    {"release-after-draw", check_release_after_draw},
    {"pass-through", check_pass_through},
    {"swap-chain-detach", check_swap_chain_detach},
};

int main(int argc, char **argv)
{
    D3D_FEATURE_LEVEL level = D3D_FEATURE_LEVEL_11_0;
    ID3D11DeviceContext *context;
    ID3D11Device *device;
    unsigned int i;
    HRESULT hr;

    for (i = 0; i < ARRAYSIZE(cases); i++)
        if (argc == 2 && !strcmp(argv[1], cases[i].name))
            break;
    if (i == ARRAYSIZE(cases))
    {
        printf(PREFIX "usage: dxmt.exe bytecode|swap-state|deferred|depth-bias|geometry-index|tessellation-index"
                      "|release-after-draw|pass-through|swap-chain-detach\n");
        return 4;
    }

    hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, &level, 1, D3D11_SDK_VERSION, &device,
                           NULL, &context);
    if (hr == DXGI_ERROR_UNSUPPORTED || hr == DXGI_ERROR_NOT_FOUND)
    {
        printf(PREFIX "no D3D11 device, %#lx\n", (unsigned long)hr);
        return 2;
    }
    if (FAILED(hr))
    {
        printf(PREFIX "D3D11CreateDevice failed, %#lx\n", (unsigned long)hr);
        return 3;
    }
    return cases[i].run(device, context);
}
