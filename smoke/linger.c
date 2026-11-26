/* The program bundle-wine.sh builds for x86_64 to check that the process a
 * launcher names in WINE_COMPATDB_APP_PID stays after it exits while a
 * program it started runs, as a game's own launcher does when it starts the
 * game and exits.
 *
 * linger.exe starts "linger.exe child <parent>", where <parent> is an
 * inherited handle of its own process, says so and exits with 42. The child
 * waits for <parent> with a time limit, checks its exit code, waits CHILD_MS
 * more and exits with 0. Its two lines tell the parent's end apart from that
 * of its unix process:
 *
 * - "parent exited with 42": the Windows process ended normally, its handle
 *   signalled and its exit code set, whether or not its unix process stays.
 * - "child exits": the last thing the child prints. When wine returns before
 *   it is in the log, the unix process did not stay for the child.
 *
 * Every wait has a time limit, so a failure never hangs. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
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

#define PARENT_WAIT_MS 5000
#define CHILD_MS 2000
#define PARENT_EXIT_CODE 42

static int child(HANDLE parent)
{
    DWORD start = GetTickCount(), code = 0;

    if (WaitForSingleObject(parent, PARENT_WAIT_MS) != WAIT_OBJECT_0)
    {
        printf("wine-build smoke test: %s linger child failed: the parent did not end within %d ms\n",
               ARCH, PARENT_WAIT_MS);
        return 1;
    }
    if (!GetExitCodeProcess(parent, &code) || code != PARENT_EXIT_CODE)
    {
        printf("wine-build smoke test: %s linger child failed: the parent exited with %lu\n",
               ARCH, code);
        return 1;
    }
    printf("wine-build smoke test: %s linger child: parent exited with %lu, waited %lu ms\n",
           ARCH, code, GetTickCount() - start);
    fflush(stdout);
    Sleep(CHILD_MS);
    printf("wine-build smoke test: %s linger child exits\n", ARCH);
    fflush(stdout);
    return 0;
}

int main(int argc, char **argv)
{
    char exe[MAX_PATH], cmd[MAX_PATH + 64];
    STARTUPINFOA si = { sizeof(si) };
    PROCESS_INFORMATION pi;
    HANDLE self, out;

    if (argc == 3 && !strcmp(argv[1], "child"))
        return child((HANDLE)(ULONG_PTR)strtoull(argv[2], NULL, 0));

    if (!GetModuleFileNameA(NULL, exe, sizeof(exe))
        || !DuplicateHandle(GetCurrentProcess(), GetCurrentProcess(), GetCurrentProcess(), &self,
                            SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, TRUE, 0)
        || !DuplicateHandle(GetCurrentProcess(), GetStdHandle(STD_OUTPUT_HANDLE),
                            GetCurrentProcess(), &out, 0, TRUE, DUPLICATE_SAME_ACCESS))
    {
        printf("wine-build smoke test: %s linger parent failed: error %lu\n", ARCH, GetLastError());
        return 1;
    }
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = si.hStdError = out;
    snprintf(cmd, sizeof(cmd), "\"%s\" child 0x%llx", exe, (unsigned long long)(ULONG_PTR)self);
    if (!CreateProcessA(exe, cmd, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi))
    {
        printf("wine-build smoke test: %s linger parent failed: CreateProcess error %lu\n", ARCH,
               GetLastError());
        return 1;
    }
    printf("wine-build smoke test: %s linger parent started the child\n", ARCH);
    fflush(stdout);
    return PARENT_EXIT_CODE;
}
