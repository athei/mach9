/* The program bundle-wine.sh builds for aarch64, the architecture of the
 * bundle's winedbg, and for i686, to check that a 64-bit debugger attaches to a
 * running 32-bit (WoW64) process and detaches again without killing it. The
 * break-in thread starts at the debugger's DbgUiRemoteBreakin, which is above
 * 4 GB, so the WoW64 process has to run it on its 64-bit context instead of
 * in the 32-bit simulation.
 *
 * attach.exe <target.exe> starts "<target.exe> target <ready> <quit>", where
 * the two are handles of events it inherits, and attaches to it once the
 * target set <ready>. It then expects one breakpoint, at an address above
 * 4 GB, and the exit of the thread that raised it, detaches, sets <quit> and
 * waits for the target to exit. It prints one line and exits with 0 when every
 * step worked and the target exited with 0, else it prints what failed, ends
 * the target and exits with 1. Every wait has a time limit, and the target
 * exits with 2 when <quit> does not come in time, so a failure never hangs. */

#include <stdarg.h>
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

#define WAIT_MS 5000
#define TARGET_WAIT_MS 10000

static PROCESS_INFORMATION pi;

static void fail(const char *format, ...)
{
    va_list args;

    printf("wine-build smoke test: %s attach failed: ", ARCH);
    va_start(args, format);
    vprintf(format, args);
    va_end(args);
    printf("\n");
    fflush(stdout);
    if (pi.hProcess) TerminateProcess(pi.hProcess, 1);
    ExitProcess(1);
}

static int target(char **argv)
{
    HANDLE ready = (HANDLE)(ULONG_PTR)strtoul(argv[2], NULL, 0);
    HANDLE quit = (HANDLE)(ULONG_PTR)strtoul(argv[3], NULL, 0);

    SetEvent(ready);
    return WaitForSingleObject(quit, TARGET_WAIT_MS) == WAIT_OBJECT_0 ? 0 : 2;
}

static int debugger(const char *path)
{
    SECURITY_ATTRIBUTES sa = { sizeof(sa), NULL, TRUE };
    STARTUPINFOA si = { sizeof(si) };
    EXCEPTION_RECORD *rec;
    DEBUG_EVENT ev;
    HANDLE ready, quit;
    DWORD breakin_tid = 0, code = ~0u;
    void *breakin_addr = NULL;
    BOOL wow64 = FALSE, done = FALSE;
    char cmd[MAX_PATH + 64];

    ready = CreateEventA(&sa, TRUE, FALSE, NULL);
    quit = CreateEventA(&sa, TRUE, FALSE, NULL);
    if (!ready || !quit) fail("CreateEvent failed, error %lu", GetLastError());
    snprintf(cmd, sizeof(cmd), "\"%s\" target %lu %lu", path,
             (unsigned long)(ULONG_PTR)ready, (unsigned long)(ULONG_PTR)quit);
    if (!CreateProcessA(NULL, cmd, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi))
        fail("cannot start %s, error %lu", path, GetLastError());
    if (!IsWow64Process(pi.hProcess, &wow64) || !wow64)
        fail("%s is not a WoW64 process", path);
    if (WaitForSingleObject(ready, WAIT_MS))
        fail("the target was not ready within %u ms", WAIT_MS);

    if (!DebugActiveProcess(pi.dwProcessId))
        fail("DebugActiveProcess failed, error %lu", GetLastError());
    while (!done)
    {
        if (!WaitForDebugEvent(&ev, WAIT_MS))
            fail("no %s within %u ms", breakin_tid ? "exit of the break-in thread" : "break-in",
                 WAIT_MS);
        switch (ev.dwDebugEventCode)
        {
        case CREATE_PROCESS_DEBUG_EVENT:
            if (ev.u.CreateProcessInfo.hFile) CloseHandle(ev.u.CreateProcessInfo.hFile);
            break;
        case LOAD_DLL_DEBUG_EVENT:
            if (ev.u.LoadDll.hFile) CloseHandle(ev.u.LoadDll.hFile);
            break;
        case EXCEPTION_DEBUG_EVENT:
            rec = &ev.u.Exception.ExceptionRecord;
            if (breakin_tid || rec->ExceptionCode != EXCEPTION_BREAKPOINT ||
                (ULONG_PTR)rec->ExceptionAddress <= 0xffffffff)
                fail("exception %08lx at %p on thread %04lx, expected one breakpoint above 4 GB",
                     rec->ExceptionCode, rec->ExceptionAddress, ev.dwThreadId);
            breakin_tid = ev.dwThreadId;
            breakin_addr = rec->ExceptionAddress;
            break;
        case EXIT_THREAD_DEBUG_EVENT:
            if (breakin_tid && ev.dwThreadId == breakin_tid) done = TRUE;
            break;
        case EXIT_PROCESS_DEBUG_EVENT:
            fail("the target exited with %lu while attached", ev.u.ExitProcess.dwExitCode);
        }
        if (!ContinueDebugEvent(ev.dwProcessId, ev.dwThreadId, DBG_CONTINUE))
            fail("ContinueDebugEvent failed, error %lu", GetLastError());
    }
    if (!DebugActiveProcessStop(pi.dwProcessId))
        fail("DebugActiveProcessStop failed, error %lu", GetLastError());

    SetEvent(quit);
    if (WaitForSingleObject(pi.hProcess, WAIT_MS))
        fail("the target did not exit within %u ms after the detach", WAIT_MS);
    if (!GetExitCodeProcess(pi.hProcess, &code) || code)
        fail("the target exited with %lu after the detach", code);
    printf("wine-build smoke test: %s attached to the WoW64 process, broke in at %p and detached\n",
           ARCH, breakin_addr);
    return 0;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc == 4 && !strcmp(argv[1], "target")) return target(argv);
    if (argc == 2) return debugger(argv[1]);
    printf("usage: attach.exe <target.exe>\n");
    return 1;
}
