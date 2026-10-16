/* The wined3d program bundle-wine.sh builds for each x86 PE architecture of a
 * bundle with Vulkan and runs under the bundled wine, once with wined3d's
 * default renderer and once with a compatdb rule that puts wined3d on
 * Vulkan. It creates a Direct3D device for a hidden window, which shows that
 * the renderer works; it prints the adapter it got, and one line per step
 * that fails.
 *
 * Built as is, it uses Direct3D 8, which always goes through wined3d. Wine
 * builds no 64-bit d3d8.dll ("configure: Disable d3d8.dll on 64-bit
 * targets."), so the x86_64 build defines SMOKE_D3D9 and uses Direct3D 9
 * instead, which goes through wined3d when compatdb gives the process the
 * wined3d d3d9 tree. */

#include <stdio.h>
#include <windows.h>

#ifdef SMOKE_D3D9
#include <d3d9.h>
#define API "d3d9"
#define DLL "d3d9.dll"
#define CREATE "Direct3DCreate9"
typedef IDirect3D9 D3D;
typedef IDirect3DDevice9 DEVICE;
typedef D3DADAPTER_IDENTIFIER9 IDENTIFIER;
#define D3D_GetAdapterIdentifier IDirect3D9_GetAdapterIdentifier
#define D3D_CreateDevice IDirect3D9_CreateDevice
#define D3D_Release IDirect3D9_Release
#define DEVICE_Release IDirect3DDevice9_Release
#else
#include <d3d8.h>
#define API "d3d8"
#define DLL "d3d8.dll"
#define CREATE "Direct3DCreate8"
typedef IDirect3D8 D3D;
typedef IDirect3DDevice8 DEVICE;
typedef D3DADAPTER_IDENTIFIER8 IDENTIFIER;
#define D3D_GetAdapterIdentifier IDirect3D8_GetAdapterIdentifier
#define D3D_CreateDevice IDirect3D8_CreateDevice
#define D3D_Release IDirect3D8_Release
#define DEVICE_Release IDirect3DDevice8_Release
#endif

#if defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

typedef D3D *(WINAPI *PFN_Create)(UINT);

int main(void)
{
    D3DPRESENT_PARAMETERS params = {0};
    IDENTIFIER id = {0};
    PFN_Create create;
    DEVICE *device;
    HMODULE module;
    HRESULT hr;
    HWND window;
    D3D *d3d;

    if (!(module = LoadLibraryA(DLL)))
    {
        printf("wine-build smoke test: %s cannot load %s, error %lu\n", ARCH, DLL, GetLastError());
        return 1;
    }
    if (!(create = (PFN_Create)(void *)GetProcAddress(module, CREATE)))
    {
        printf("wine-build smoke test: %s %s has no %s\n", ARCH, DLL, CREATE);
        return 1;
    }
    if (!(d3d = create(D3D_SDK_VERSION)))
    {
        printf("wine-build smoke test: %s %s failed: wined3d has no adapter\n", ARCH, CREATE);
        return 1;
    }
    if (FAILED(hr = D3D_GetAdapterIdentifier(d3d, D3DADAPTER_DEFAULT, 0, &id)))
    {
        printf("wine-build smoke test: %s GetAdapterIdentifier failed, %#lx\n", ARCH, hr);
        return 1;
    }
    window = CreateWindowA("static", "wine-build smoke test", WS_OVERLAPPEDWINDOW, 0, 0, 64, 64,
                           NULL, NULL, NULL, NULL);
    if (!window)
    {
        printf("wine-build smoke test: %s cannot create a window, error %lu\n", ARCH,
               GetLastError());
        return 1;
    }
    params.Windowed = TRUE;
    params.SwapEffect = D3DSWAPEFFECT_DISCARD;
    params.BackBufferWidth = 64;
    params.BackBufferHeight = 64;
    params.BackBufferFormat = D3DFMT_UNKNOWN;
    params.hDeviceWindow = window;
    hr = D3D_CreateDevice(d3d, D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, window,
                          D3DCREATE_SOFTWARE_VERTEXPROCESSING, &params, &device);
    if (FAILED(hr))
    {
        printf("wine-build smoke test: %s %s CreateDevice on %.512s failed, %#lx\n", ARCH, API,
               id.Description, hr);
        return 1;
    }
    printf("wine-build smoke test: %s %s device on %.512s\n", ARCH, API, id.Description);
    DEVICE_Release(device);
    D3D_Release(d3d);
    DestroyWindow(window);
    return 0;
}
