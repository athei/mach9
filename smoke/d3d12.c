/* The Direct3D 12 program bundle-wine.sh builds for each PE architecture it
 * tests and runs under the bundled wine. D3D12 is vkd3d-proton on
 * KosmicKrisp, and its swapchains are presented by DXMT's dxgi.dll, both from
 * the dxmt tree compatdb.so gives every process by default. The program goes
 * through what a game does: it prints the first DXGI adapter and its LUID,
 * creates a device on that adapter and one without an adapter, checks that
 * both report the adapter's LUID, creates a direct queue and a flip-model
 * swapchain with two buffers for a visible window, clears a buffer through a
 * command list and reads one pixel of it back, presents several frames with
 * a fence wait after each, resizes the buffers and presents again, then does
 * the same with a frame latency waitable object. It asks the swapchain for
 * its output, frame statistics and fullscreen state, and checks that a
 * swapchain on a compute queue and one for the desktop window are refused as
 * on Windows.
 *
 * It prints a few lines on the way and, for the step that fails, one line
 * and a distinct exit status:
 *
 *   2  dxgi.dll or CreateDXGIFactory1 missing   15  frames of the first swapchain
 *   3  CreateDXGIFactory1 failed                16  GetContainingOutput
 *   4  no DXGI adapter                          17  GetFrameStatistics or
 *   5  d3d12.dll or D3D12CreateDevice missing       GetFullscreenState
 *   6  D3D12CreateDevice on the adapter         18  ResizeBuffers or GetDesc1
 *   7  D3D12CreateDevice without an adapter     19  buffers after the resize
 *   8  a device reports another LUID            20  frames after the resize
 *   9  direct queue                             21  waitable swapchain
 *  10  window                                   22  frame latency handle
 *  11  CreateSwapChainForHwnd                   23  handle not signaled in time
 *  12  IDXGISwapChain3 or IDXGISwapChain4       24  frames of the waitable one
 *  13  GetBuffer                                25  compute queue
 *  14  allocator, command list, fence, heap     26  compute queue swapchain
 *      or readback buffer                           not refused
 *                                               27  desktop window swapchain
 *                                                   not refused
 *
 * A failure in a frame (recording, presenting, the fence wait, or a pixel
 * read back that is not the clear color) exits with the status of the
 * frames it belongs to; the line it prints says which.
 *
 * The first device creation is the step that fails on a Mac without a Metal
 * 4 GPU, where KosmicKrisp has no device to offer. */

#define COBJMACROS
#define WIDL_C_INLINE_WRAPPERS
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <dxgi1_6.h>

#if defined(__aarch64__) && !defined(__arm64ec__)
#define ARCH "aarch64"
#elif defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

#define WIDTH 64
#define HEIGHT 64
#define FRAMES 6
#define FORMAT DXGI_FORMAT_B8G8R8A8_UNORM

typedef HRESULT (WINAPI *PFN_CreateDXGIFactory1)(REFIID, void **);

/* The clear color, and the bytes it gives in FORMAT's B, G, R, A order: each
 * channel is a multiple of 1/255, so the conversion is exact up to rounding. */
static const float clear_color[4] = { 51 / 255.0f, 102 / 255.0f, 204 / 255.0f, 1.0f };
static const BYTE clear_bytes[4] = { 204, 102, 51, 255 };

static ID3D12Device *device;
static ID3D12CommandQueue *queue;
static ID3D12CommandAllocator *allocator;
static ID3D12GraphicsCommandList *list;
static ID3D12Fence *fence;
static UINT64 fence_value;
static HANDLE fence_event;
static ID3D12DescriptorHeap *rtv_heap;
static ID3D12Resource *readback;

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

/* Print the failure <fmt> and exit with <status>. */
static void fail(int status, const char *fmt, ...)
{
    va_list args;

    printf("wine-build smoke test: %s ", ARCH);
    va_start(args, fmt);
    vprintf(fmt, args);
    va_end(args);
    printf("\n");
    fflush(stdout);
    exit(status);
}

static void pump_messages(void)
{
    MSG msg;

    while (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE))
    {
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
}

static HWND create_window(const char *title)
{
    HWND window = CreateWindowA("static", title, WS_OVERLAPPEDWINDOW | WS_VISIBLE, 0, 0,
                                WIDTH * 2, HEIGHT * 2, NULL, NULL, NULL, NULL);

    if (!window) fail(10, "cannot create a window, error %lu", GetLastError());
    ShowWindow(window, SW_SHOW);
    UpdateWindow(window);
    pump_messages();
    return window;
}

/* Signal the fence on the queue and wait until the GPU passed it, exiting
 * with <status> on a failure. */
static void wait_gpu(int status)
{
    HRESULT hr;

    if (FAILED(hr = ID3D12CommandQueue_Signal(queue, fence, ++fence_value)))
        fail(status, "Signal failed, %#lx", hr);
    if (ID3D12Fence_GetCompletedValue(fence) >= fence_value) return;
    if (FAILED(hr = ID3D12Fence_SetEventOnCompletion(fence, fence_value, fence_event)))
        fail(status, "SetEventOnCompletion failed, %#lx", hr);
    if (WaitForSingleObject(fence_event, 5000) != WAIT_OBJECT_0)
        fail(status, "the fence did not reach %llu within 5 seconds",
             (unsigned long long)fence_value);
}

static void transition(ID3D12Resource *resource, D3D12_RESOURCE_STATES before,
                       D3D12_RESOURCE_STATES after)
{
    D3D12_RESOURCE_BARRIER barrier = {0};

    barrier.Type = D3D12_RESOURCE_BARRIER_TYPE_TRANSITION;
    barrier.Transition.pResource = resource;
    barrier.Transition.Subresource = D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES;
    barrier.Transition.StateBefore = before;
    barrier.Transition.StateAfter = after;
    ID3D12GraphicsCommandList_ResourceBarrier(list, 1, &barrier);
}

/* Clear the current back buffer of <swapchain>, whose buffers are <buffers>,
 * present it with a sync interval of 1 and wait for the GPU. With <check>,
 * also copy its first pixel to the readback buffer and exit unless it is the
 * clear color. Every failure exits with <status>. */
static void frame(IDXGISwapChain3 *swapchain, ID3D12Resource **buffers, int check, int status)
{
    D3D12_TEXTURE_COPY_LOCATION dst = {0}, src = {0};
    D3D12_CPU_DESCRIPTOR_HANDLE rtv;
    D3D12_BOX box = { 0, 0, 0, 1, 1, 1 };
    D3D12_RANGE range = { 0, 4 }, written = { 0, 0 };
    ID3D12Resource *buffer;
    BYTE *pixel;
    HRESULT hr;
    int i;

    buffer = buffers[IDXGISwapChain3_GetCurrentBackBufferIndex(swapchain)];
    if (FAILED(hr = ID3D12CommandAllocator_Reset(allocator)))
        fail(status, "ID3D12CommandAllocator::Reset failed, %#lx", hr);
    if (FAILED(hr = ID3D12GraphicsCommandList_Reset(list, allocator, NULL)))
        fail(status, "ID3D12GraphicsCommandList::Reset failed, %#lx", hr);
    transition(buffer, D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_RENDER_TARGET);
    rtv = ID3D12DescriptorHeap_GetCPUDescriptorHandleForHeapStart(rtv_heap);
    ID3D12Device_CreateRenderTargetView(device, buffer, NULL, rtv);
    ID3D12GraphicsCommandList_ClearRenderTargetView(list, rtv, clear_color, 0, NULL);
    if (check)
    {
        transition(buffer, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_COPY_SOURCE);
        dst.pResource = readback;
        dst.Type = D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
        dst.PlacedFootprint.Footprint.Format = FORMAT;
        dst.PlacedFootprint.Footprint.Width = 1;
        dst.PlacedFootprint.Footprint.Height = 1;
        dst.PlacedFootprint.Footprint.Depth = 1;
        dst.PlacedFootprint.Footprint.RowPitch = D3D12_TEXTURE_DATA_PITCH_ALIGNMENT;
        src.pResource = buffer;
        src.Type = D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
        src.SubresourceIndex = 0;
        ID3D12GraphicsCommandList_CopyTextureRegion(list, &dst, 0, 0, 0, &src, &box);
        transition(buffer, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_PRESENT);
    }
    else
    {
        transition(buffer, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PRESENT);
    }
    if (FAILED(hr = ID3D12GraphicsCommandList_Close(list)))
        fail(status, "ID3D12GraphicsCommandList::Close failed, %#lx", hr);
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, (ID3D12CommandList **)&list);
    if (FAILED(hr = IDXGISwapChain3_Present(swapchain, 1, 0)))
        fail(status, "Present failed, %#lx", hr);
    wait_gpu(status);
    pump_messages();
    if (!check) return;
    if (FAILED(hr = ID3D12Resource_Map(readback, 0, &range, (void **)&pixel)))
        fail(status, "mapping the readback buffer failed, %#lx", hr);
    for (i = 0; i < 4; i++)
    {
        if (abs(pixel[i] - clear_bytes[i]) > 1)
            fail(status, "read back %02x%02x%02x%02x (BGRA), not the clear color %02x%02x%02x%02x",
                 pixel[0], pixel[1], pixel[2], pixel[3],
                 clear_bytes[0], clear_bytes[1], clear_bytes[2], clear_bytes[3]);
    }
    ID3D12Resource_Unmap(readback, 0, &written);
}

static void get_buffers(IDXGISwapChain3 *swapchain, ID3D12Resource **buffers, int status)
{
    HRESULT hr;
    UINT i;

    for (i = 0; i < 2; i++)
    {
        if (FAILED(hr = IDXGISwapChain3_GetBuffer(swapchain, i, &IID_ID3D12Resource,
                                                  (void **)&buffers[i])))
            fail(status, "GetBuffer(%u) failed, %#lx", i, hr);
    }
}

static void release_buffers(ID3D12Resource **buffers)
{
    ID3D12Resource_Release(buffers[0]);
    ID3D12Resource_Release(buffers[1]);
    buffers[0] = buffers[1] = NULL;
}

static void swapchain_desc(DXGI_SWAP_CHAIN_DESC1 *desc, UINT flags)
{
    memset(desc, 0, sizeof(*desc));
    desc->Width = WIDTH;
    desc->Height = HEIGHT;
    desc->Format = FORMAT;
    desc->SampleDesc.Count = 1;
    desc->BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    desc->BufferCount = 2;
    desc->Scaling = DXGI_SCALING_STRETCH;
    desc->SwapEffect = DXGI_SWAP_EFFECT_FLIP_DISCARD;
    desc->Flags = flags;
}

/* <text> as UTF-8 in <buffer> of <size> bytes, for printing. */
static const char *utf8(const WCHAR *text, char *buffer, int size)
{
    if (!WideCharToMultiByte(CP_UTF8, 0, text, -1, buffer, size, NULL, NULL))
        strcpy(buffer, "(unnamed)");
    return buffer;
}

static void check_luid(const char *which, LUID luid, LUID want)
{
    if (luid.LowPart != want.LowPart || luid.HighPart != want.HighPart)
        fail(8, "the device %s reports the LUID %08lx:%08lx, the adapter has %08lx:%08lx", which,
             luid.HighPart, luid.LowPart, want.HighPart, want.LowPart);
}

int main(void)
{
    D3D12_COMMAND_QUEUE_DESC queue_desc = { D3D12_COMMAND_LIST_TYPE_DIRECT, 0,
                                            D3D12_COMMAND_QUEUE_FLAG_NONE, 0 };
    D3D12_DESCRIPTOR_HEAP_DESC heap_desc = { D3D12_DESCRIPTOR_HEAP_TYPE_RTV, 1,
                                             D3D12_DESCRIPTOR_HEAP_FLAG_NONE, 0 };
    D3D12_HEAP_PROPERTIES heap_props = { D3D12_HEAP_TYPE_READBACK, D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                                         D3D12_MEMORY_POOL_UNKNOWN, 0, 0 };
    D3D12_RESOURCE_DESC buffer_desc = {0};
    PFN_D3D12_CREATE_DEVICE create_device;
    PFN_CreateDXGIFactory1 create_factory;
    IDXGISwapChain1 *swapchain1, *refused;
    IDXGISwapChain3 *swapchain3;
    IDXGISwapChain4 *swapchain4;
    ID3D12Resource *buffers[2];
    ID3D12CommandQueue *compute;
    DXGI_FRAME_STATISTICS stats;
    DXGI_SWAP_CHAIN_DESC1 desc;
    DXGI_ADAPTER_DESC1 adapter_desc;
    DXGI_OUTPUT_DESC output_desc;
    IDXGIAdapter1 *adapter;
    IDXGIFactory2 *factory;
    ID3D12Device *device2;
    IDXGIOutput *output;
    HMODULE dxgi, d3d12;
    char name[512], output_name[128];
    HRESULT hr, stats_hr;
    HANDLE latency;
    BOOL fullscreen;
    HWND window;
    int i;

    /* DXGI and the adapter. */
    if (!(dxgi = LoadLibraryA("dxgi.dll")))
        fail(2, "cannot load dxgi.dll, error %lu", GetLastError());
    if (!(create_factory = (PFN_CreateDXGIFactory1)(void *)GetProcAddress(dxgi, "CreateDXGIFactory1")))
        fail(2, "dxgi.dll has no CreateDXGIFactory1");
    if (FAILED(hr = create_factory(&IID_IDXGIFactory2, (void **)&factory)))
        fail(3, "CreateDXGIFactory1 failed, %#lx", hr);
    if (FAILED(hr = IDXGIFactory2_EnumAdapters1(factory, 0, &adapter)))
        fail(4, "EnumAdapters1(0) failed, %#lx", hr);
    if (FAILED(hr = IDXGIAdapter1_GetDesc1(adapter, &adapter_desc)))
        fail(4, "IDXGIAdapter1::GetDesc1 failed, %#lx", hr);
    utf8(adapter_desc.Description, name, sizeof(name));
    say("dxgi adapter %s, LUID %08lx:%08lx", name, adapter_desc.AdapterLuid.HighPart,
        adapter_desc.AdapterLuid.LowPart);

    /* The devices, with and without the adapter. */
    if (!(d3d12 = LoadLibraryA("d3d12.dll")))
        fail(5, "cannot load d3d12.dll, error %lu", GetLastError());
    if (!(create_device = (PFN_D3D12_CREATE_DEVICE)(void *)GetProcAddress(d3d12, "D3D12CreateDevice")))
        fail(5, "d3d12.dll has no D3D12CreateDevice");
    if (FAILED(hr = create_device((IUnknown *)adapter, D3D_FEATURE_LEVEL_11_0, &IID_ID3D12Device,
                                  (void **)&device)))
        fail(6, "D3D12CreateDevice on the dxgi adapter failed, %#lx", hr);
    if (FAILED(hr = create_device(NULL, D3D_FEATURE_LEVEL_11_0, &IID_ID3D12Device,
                                  (void **)&device2)))
        fail(7, "D3D12CreateDevice without an adapter failed, %#lx", hr);
    check_luid("on the adapter", ID3D12Device_GetAdapterLuid(device), adapter_desc.AdapterLuid);
    check_luid("without an adapter", ID3D12Device_GetAdapterLuid(device2), adapter_desc.AdapterLuid);
    ID3D12Device_Release(device2);
    say("d3d12 devices with and without the adapter, both on its LUID");

    /* The queue and what each frame records with. */
    if (FAILED(hr = ID3D12Device_CreateCommandQueue(device, &queue_desc, &IID_ID3D12CommandQueue,
                                                    (void **)&queue)))
        fail(9, "CreateCommandQueue(DIRECT) failed, %#lx", hr);
    if (FAILED(hr = ID3D12Device_CreateCommandAllocator(device, D3D12_COMMAND_LIST_TYPE_DIRECT,
                                                        &IID_ID3D12CommandAllocator,
                                                        (void **)&allocator)))
        fail(14, "CreateCommandAllocator failed, %#lx", hr);
    if (FAILED(hr = ID3D12Device_CreateCommandList(device, 0, D3D12_COMMAND_LIST_TYPE_DIRECT,
                                                   allocator, NULL,
                                                   &IID_ID3D12GraphicsCommandList,
                                                   (void **)&list)))
        fail(14, "CreateCommandList failed, %#lx", hr);
    ID3D12GraphicsCommandList_Close(list);
    if (FAILED(hr = ID3D12Device_CreateFence(device, 0, D3D12_FENCE_FLAG_NONE, &IID_ID3D12Fence,
                                             (void **)&fence)))
        fail(14, "CreateFence failed, %#lx", hr);
    if (!(fence_event = CreateEventA(NULL, FALSE, FALSE, NULL)))
        fail(14, "CreateEvent failed, error %lu", GetLastError());
    if (FAILED(hr = ID3D12Device_CreateDescriptorHeap(device, &heap_desc,
                                                      &IID_ID3D12DescriptorHeap,
                                                      (void **)&rtv_heap)))
        fail(14, "CreateDescriptorHeap(RTV) failed, %#lx", hr);
    buffer_desc.Dimension = D3D12_RESOURCE_DIMENSION_BUFFER;
    buffer_desc.Width = D3D12_TEXTURE_DATA_PITCH_ALIGNMENT;
    buffer_desc.Height = 1;
    buffer_desc.DepthOrArraySize = 1;
    buffer_desc.MipLevels = 1;
    buffer_desc.SampleDesc.Count = 1;
    buffer_desc.Layout = D3D12_TEXTURE_LAYOUT_ROW_MAJOR;
    if (FAILED(hr = ID3D12Device_CreateCommittedResource(device, &heap_props, D3D12_HEAP_FLAG_NONE,
                                                         &buffer_desc,
                                                         D3D12_RESOURCE_STATE_COPY_DEST, NULL,
                                                         &IID_ID3D12Resource,
                                                         (void **)&readback)))
        fail(14, "creating the readback buffer failed, %#lx", hr);

    /* A flip-model swapchain for a visible window. */
    window = create_window("wine-build smoke test");
    swapchain_desc(&desc, 0);
    if (FAILED(hr = IDXGIFactory2_CreateSwapChainForHwnd(factory, (IUnknown *)queue, window, &desc,
                                                         NULL, NULL, &swapchain1)))
        fail(11, "CreateSwapChainForHwnd failed, %#lx", hr);
    if (FAILED(hr = IDXGISwapChain1_QueryInterface(swapchain1, &IID_IDXGISwapChain3,
                                                   (void **)&swapchain3)))
        fail(12, "the swapchain has no IDXGISwapChain3, %#lx", hr);
    if (FAILED(hr = IDXGISwapChain1_QueryInterface(swapchain1, &IID_IDXGISwapChain4,
                                                   (void **)&swapchain4)))
        fail(12, "the swapchain has no IDXGISwapChain4, %#lx", hr);
    get_buffers(swapchain3, buffers, 13);
    frame(swapchain3, buffers, 1, 15);
    for (i = 1; i < FRAMES; i++) frame(swapchain3, buffers, 0, 15);
    say("swapchain %ux%u presents %d frames, the clear color reads back", WIDTH, HEIGHT, FRAMES);

    if (FAILED(hr = IDXGISwapChain1_GetContainingOutput(swapchain1, &output)))
        fail(16, "GetContainingOutput failed, %#lx", hr);
    if (FAILED(hr = IDXGIOutput_GetDesc(output, &output_desc)))
        fail(16, "IDXGIOutput::GetDesc failed, %#lx", hr);
    IDXGIOutput_Release(output);
    /* The first call, and the first after a mode change, may say the
     * statistics are disjoint; that is a valid answer too. */
    stats_hr = IDXGISwapChain1_GetFrameStatistics(swapchain1, &stats);
    if (FAILED(stats_hr) && stats_hr != DXGI_ERROR_FRAME_STATISTICS_DISJOINT)
        fail(17, "GetFrameStatistics failed, %#lx", stats_hr);
    output = NULL;
    if (FAILED(hr = IDXGISwapChain1_GetFullscreenState(swapchain1, &fullscreen, &output)))
        fail(17, "GetFullscreenState failed, %#lx", hr);
    if (fullscreen) fail(17, "GetFullscreenState says fullscreen for a windowed swapchain");
    if (output) IDXGIOutput_Release(output);
    say("output %s, frame statistics %#lx (present count %u), windowed",
        utf8(output_desc.DeviceName, output_name, sizeof(output_name)), stats_hr,
        SUCCEEDED(stats_hr) ? stats.PresentCount : 0);

    /* The same swapchain with other buffers. */
    release_buffers(buffers);
    if (FAILED(hr = IDXGISwapChain4_ResizeBuffers(swapchain4, 0, WIDTH * 2, HEIGHT * 2,
                                                  DXGI_FORMAT_UNKNOWN, 0)))
        fail(18, "ResizeBuffers failed, %#lx", hr);
    if (FAILED(hr = IDXGISwapChain1_GetDesc1(swapchain1, &desc)))
        fail(18, "GetDesc1 failed, %#lx", hr);
    if (desc.Width != WIDTH * 2 || desc.Height != HEIGHT * 2)
        fail(18, "the resized swapchain is %ux%u, not %ux%u", desc.Width, desc.Height,
             WIDTH * 2, HEIGHT * 2);
    get_buffers(swapchain3, buffers, 19);
    frame(swapchain3, buffers, 1, 20);
    for (i = 1; i < FRAMES; i++) frame(swapchain3, buffers, 0, 20);
    say("resized to %ux%u, presents again", desc.Width, desc.Height);
    release_buffers(buffers);
    IDXGISwapChain4_Release(swapchain4);
    IDXGISwapChain3_Release(swapchain3);
    IDXGISwapChain1_Release(swapchain1);
    DestroyWindow(window);

    /* A swapchain with a frame latency waitable object, on a window of its
     * own: each frame waits for the handle before it renders. */
    window = create_window("wine-build smoke test, waitable");
    swapchain_desc(&desc, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT);
    if (FAILED(hr = IDXGIFactory2_CreateSwapChainForHwnd(factory, (IUnknown *)queue, window, &desc,
                                                         NULL, NULL, &swapchain1)))
        fail(21, "CreateSwapChainForHwnd with a waitable object failed, %#lx", hr);
    if (FAILED(hr = IDXGISwapChain1_QueryInterface(swapchain1, &IID_IDXGISwapChain3,
                                                   (void **)&swapchain3)))
        fail(21, "the waitable swapchain has no IDXGISwapChain3, %#lx", hr);
    if (FAILED(hr = IDXGISwapChain3_SetMaximumFrameLatency(swapchain3, 1)))
        fail(22, "SetMaximumFrameLatency(1) failed, %#lx", hr);
    if (!(latency = IDXGISwapChain3_GetFrameLatencyWaitableObject(swapchain3)))
        fail(22, "GetFrameLatencyWaitableObject returned no handle");
    get_buffers(swapchain3, buffers, 21);
    for (i = 0; i < FRAMES; i++)
    {
        if (WaitForSingleObjectEx(latency, 5000, TRUE) != WAIT_OBJECT_0)
            fail(23, "the frame latency handle was not signaled within 5 seconds in frame %d", i);
        frame(swapchain3, buffers, i == 0, 24);
    }
    say("waitable swapchain presents %d frames", FRAMES);
    CloseHandle(latency);
    release_buffers(buffers);
    IDXGISwapChain3_Release(swapchain3);
    IDXGISwapChain1_Release(swapchain1);
    DestroyWindow(window);

    /* What Windows refuses: a swapchain on a queue that cannot present, and
     * one for the desktop window. */
    window = create_window("wine-build smoke test, refused");
    queue_desc.Type = D3D12_COMMAND_LIST_TYPE_COMPUTE;
    if (FAILED(hr = ID3D12Device_CreateCommandQueue(device, &queue_desc, &IID_ID3D12CommandQueue,
                                                    (void **)&compute)))
        fail(25, "CreateCommandQueue(COMPUTE) failed, %#lx", hr);
    swapchain_desc(&desc, 0);
    hr = IDXGIFactory2_CreateSwapChainForHwnd(factory, (IUnknown *)compute, window, &desc, NULL,
                                              NULL, &refused);
    if (SUCCEEDED(hr)) IDXGISwapChain1_Release(refused);
    if (hr != DXGI_ERROR_INVALID_CALL)
        fail(26, "CreateSwapChainForHwnd on a compute queue returned %#lx, not "
             "DXGI_ERROR_INVALID_CALL", hr);
    ID3D12CommandQueue_Release(compute);
    hr = IDXGIFactory2_CreateSwapChainForHwnd(factory, (IUnknown *)queue, GetDesktopWindow(), &desc,
                                              NULL, NULL, &refused);
    if (SUCCEEDED(hr)) IDXGISwapChain1_Release(refused);
    if (hr != E_ACCESSDENIED)
        fail(27, "CreateSwapChainForHwnd for the desktop window returned %#lx, not "
             "E_ACCESSDENIED", hr);
    DestroyWindow(window);
    say("swapchains on a compute queue and for the desktop window are refused");

    ID3D12Resource_Release(readback);
    ID3D12DescriptorHeap_Release(rtv_heap);
    CloseHandle(fence_event);
    ID3D12Fence_Release(fence);
    ID3D12GraphicsCommandList_Release(list);
    ID3D12CommandAllocator_Release(allocator);
    ID3D12CommandQueue_Release(queue);
    ID3D12Device_Release(device);
    IDXGIAdapter1_Release(adapter);
    IDXGIFactory2_Release(factory);
    say("d3d12 works on %s", name);
    return 0;
}
