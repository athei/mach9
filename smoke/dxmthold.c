/* The DLL the swap-chain-detach case of smoke/dxmt.c loads, which
 * bundle-wine.sh builds beside dxmt.exe for each of its architectures as
 * dxmthold-<arch>.dll. hold() takes a reference to an object, and the DLL's
 * DLL_PROCESS_DETACH at process exit releases it, as the static destructors
 * of a game's DLL release what they hold. By then the process's other
 * threads are gone. The DLL prints the reference count the release left and
 * whether it was the last one; a release that does not return leaves the
 * process running until bundle-wine.sh's time limit.
 *
 * It writes to the standard output handle directly, not through the C
 * runtime's streams, which exit has flushed and may have closed by then. */

#define COBJMACROS
#include <stdio.h>
#include <string.h>
#include <windows.h>
#include <unknwn.h>

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

static IUnknown *held;

static void say(const char *line)
{
    DWORD written;

    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), line, strlen(line), &written, NULL);
}

__declspec(dllexport) void hold(IUnknown *object)
{
    IUnknown_AddRef(object);
    held = object;
}

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, void *reserved)
{
    char line[160];
    ULONG refcount;

    /* reserved is not NULL in the detach of process exit */
    if (reason != DLL_PROCESS_DETACH || !reserved || !held)
        return TRUE;
    refcount = IUnknown_Release(held);
    snprintf(line, sizeof(line), PREFIX "swap-chain-detach: released in the DLL detach at process exit, %lu "
             "references left\n", refcount);
    say(line);
    say(refcount ? PREFIX "swap-chain-detach check FAILED\n" : PREFIX "swap-chain-detach check ok\n");
    return TRUE;
}
