/* The console program bundle-wine.sh --arm64 builds for each PE architecture
 * and runs under the bundled wine. It prints a marker naming the
 * architecture it was compiled for, then tries dxgi.dll and d3d9.dll, which
 * come from the Direct3D trees compatdb.so picks for the process, and says
 * whether both loaded. */

#include <stdio.h>
#include <windows.h>

#if defined(__aarch64__) && !defined(__arm64ec__)
#define ARCH "aarch64"
#elif defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

int main(void)
{
    HMODULE dxgi, d3d9;

    printf("wine-build smoke test: hello from %s\n", ARCH);
    fflush(stdout);
    dxgi = LoadLibraryA("dxgi.dll");
    d3d9 = LoadLibraryA("d3d9.dll");
    if (dxgi && d3d9)
        printf("wine-build smoke test: %s loads dxgi.dll and d3d9.dll\n", ARCH);
    else
        printf("wine-build smoke test: %s cannot load dxgi.dll (%p) or d3d9.dll (%p), error %lu\n",
               ARCH, (void *)dxgi, (void *)d3d9, GetLastError());
    return 0;
}
