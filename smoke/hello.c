/* The console program bundle-wine.sh builds for each PE architecture it
 * tests and runs under the bundled wine. It prints a marker naming the
 * architecture it was compiled for, then loads each DLL named on its command
 * line (dxgi.dll and d3d9.dll without arguments), which come from the
 * Direct3D trees compatdb.so picks for the process. It prints one line
 * listing them all when every one loaded, else one line per DLL that did
 * not. */

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

int main(int argc, char **argv)
{
    static char *defaults[] = { "dxgi.dll", "d3d9.dll" };
    char **names = argv + 1;
    int count = argc - 1, failed = 0, i;

    if (!count)
    {
        names = defaults;
        count = 2;
    }
    printf("wine-build smoke test: hello from %s\n", ARCH);
    fflush(stdout);
    for (i = 0; i < count; i++)
    {
        if (!LoadLibraryA(names[i]))
        {
            printf("wine-build smoke test: %s cannot load %s, error %lu\n", ARCH, names[i],
                   GetLastError());
            failed = 1;
        }
    }
    if (!failed)
    {
        printf("wine-build smoke test: %s loads", ARCH);
        for (i = 0; i < count; i++) printf(" %s", names[i]);
        printf("\n");
    }
    return 0;
}
