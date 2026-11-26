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
                      "|release-after-draw\n");
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
