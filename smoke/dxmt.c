/* The DXMT program bundle-wine.sh builds for x86_64 and i686 and runs under
 * the bundled wine, once per case, with the case name as its argument:
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
 *
 * The two lifetime cases watch destruction through a tracker object attached
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

#if defined(__x86_64__)
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

static const struct
{
    const char *name;
    int (*run)(ID3D11Device *device, ID3D11DeviceContext *context);
} cases[] = {
    {"bytecode", check_bytecode},
    {"swap-state", check_swap_state},
    {"deferred", check_deferred},
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
        printf(PREFIX "usage: dxmt.exe bytecode|swap-state|deferred\n");
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
