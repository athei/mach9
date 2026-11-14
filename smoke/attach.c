/* The program bundle-wine.sh builds for aarch64, the architecture of the
 * bundle's winedbg, and for i686 and x86_64, to check that a 64-bit debugger
 * attaches to a running x86 process and detaches again without killing it.
 * The debugger asks for the break-in thread at its own DbgUiRemoteBreakin,
 * which neither target can run as it is. For a 32-bit (WoW64) process it is
 * above 4 GB, so the process has to run it on its 64-bit context instead of
 * in the 32-bit simulation. For an x86_64 (ARM64EC) process it is in the
 * ARM64 half of ntdll, which is not set up there and which a call from
 * ARM64EC code hands to the emulator as x86 code, so the process has to run
 * the DbgUiRemoteBreakin of its own half. The x86_64 build is also run as the
 * debugger of the i686 one: it is an ARM64EC process, so its
 * DbgUiRemoteBreakin is in the ARM64EC half, which the 64-bit side of a WoW64
 * process, the ARM64 half, cannot run.
 *
 * attach.exe <continue|pass> <target.exe> starts "<target.exe> target <ready>
 * <quit>", where the two are handles of events it inherits, and attaches to it
 * once the target set <ready>. The target has to run as an i386 or x86_64
 * process. The program then expects one breakpoint, at an address above 4 GB,
 * and the exit with 0 of the thread that raised it, detaches, sets <quit> and
 * waits for the target to exit. With continue it continues the breakpoint as
 * handled. With pass it passes it on to the target, as winedbg's pass does, so
 * that the exception dispatch of the break-in thread runs and
 * DbgUiRemoteBreakin's own handler takes it; the emulator then sees an
 * exception on a thread it never ran. Any other exception fails the run when
 * the debugger gets it. The program prints one line, which names the
 * target's architecture, and exits with 0 when every step worked and the
 * target exited with 0, else it prints what failed, ends the target and exits
 * with 1. Every wait has a time limit, and the target exits with 2 when
 * <quit> does not come in time, so a failure never hangs. */

/* for GetProcessInformation() and ProcessMachineTypeInfo */
#define _WIN32_WINNT 0x0A00
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

static int debugger(BOOL pass, const char *path)
{
    SECURITY_ATTRIBUTES sa = { sizeof(sa), NULL, TRUE };
    STARTUPINFOA si = { sizeof(si) };
    EXCEPTION_RECORD *rec;
    DEBUG_EVENT ev;
    HANDLE ready, quit;
    DWORD breakin_tid = 0, code = ~0u, status;
    void *breakin_addr = NULL;
    PROCESS_MACHINE_INFORMATION machine = { 0 };
    const char *target_arch = NULL;
    BOOL done = FALSE;
    char cmd[MAX_PATH + 64];

    ready = CreateEventA(&sa, TRUE, FALSE, NULL);
    quit = CreateEventA(&sa, TRUE, FALSE, NULL);
    if (!ready || !quit) fail("CreateEvent failed, error %lu", GetLastError());
    snprintf(cmd, sizeof(cmd), "\"%s\" target %lu %lu", path,
             (unsigned long)(ULONG_PTR)ready, (unsigned long)(ULONG_PTR)quit);
    if (!CreateProcessA(NULL, cmd, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi))
        fail("cannot start %s, error %lu", path, GetLastError());
    if (!GetProcessInformation(pi.hProcess, ProcessMachineTypeInfo, &machine, sizeof(machine)))
        fail("cannot read the machine of %s, error %lu", path, GetLastError());
    if (machine.ProcessMachine == IMAGE_FILE_MACHINE_I386) target_arch = "i686";
    else if (machine.ProcessMachine == IMAGE_FILE_MACHINE_AMD64) target_arch = "x86_64";
    else fail("%s runs as machine %04x, not as an i386 or x86_64 process", path,
              machine.ProcessMachine);
    if (WaitForSingleObject(ready, WAIT_MS))
        fail("the target was not ready within %u ms", WAIT_MS);

    if (!DebugActiveProcess(pi.dwProcessId))
        fail("DebugActiveProcess failed, error %lu", GetLastError());
    while (!done)
    {
        if (!WaitForDebugEvent(&ev, WAIT_MS))
            fail("no %s within %u ms", breakin_tid ? "exit of the break-in thread" : "break-in",
                 WAIT_MS);
        status = DBG_CONTINUE;
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
            if (breakin_tid)
                fail("exception %08lx at %p on thread %04lx after the break-in on thread %04lx",
                     rec->ExceptionCode, rec->ExceptionAddress, ev.dwThreadId, breakin_tid);
            if (rec->ExceptionCode != EXCEPTION_BREAKPOINT ||
                (ULONG_PTR)rec->ExceptionAddress <= 0xffffffff)
                fail("exception %08lx at %p on thread %04lx, expected one breakpoint above 4 GB",
                     rec->ExceptionCode, rec->ExceptionAddress, ev.dwThreadId);
            breakin_tid = ev.dwThreadId;
            breakin_addr = rec->ExceptionAddress;
            if (pass) status = DBG_EXCEPTION_NOT_HANDLED;
            break;
        case EXIT_THREAD_DEBUG_EVENT:
            if (!breakin_tid || ev.dwThreadId != breakin_tid) break;
            if (ev.u.ExitThread.dwExitCode)
                fail("the break-in thread exited with %08lx", ev.u.ExitThread.dwExitCode);
            done = TRUE;
            break;
        case EXIT_PROCESS_DEBUG_EVENT:
            fail("the target exited with %lu while attached", ev.u.ExitProcess.dwExitCode);
        }
        if (!ContinueDebugEvent(ev.dwProcessId, ev.dwThreadId, status))
            fail("ContinueDebugEvent failed, error %lu", GetLastError());
    }
    if (!DebugActiveProcessStop(pi.dwProcessId))
        fail("DebugActiveProcessStop failed, error %lu", GetLastError());

    SetEvent(quit);
    if (WaitForSingleObject(pi.hProcess, WAIT_MS))
        fail("the target did not exit within %u ms after the detach", WAIT_MS);
    if (!GetExitCodeProcess(pi.hProcess, &code) || code)
        fail("the target exited with %lu after the detach", code);
    printf("wine-build smoke test: %s attached to the %s process, %s the break-in at %p and detached\n",
           ARCH, target_arch, pass ? "passed on" : "continued", breakin_addr);
    return 0;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc == 4 && !strcmp(argv[1], "target")) return target(argv);
    if (argc == 3 && !strcmp(argv[1], "continue")) return debugger(FALSE, argv[2]);
    if (argc == 3 && !strcmp(argv[1], "pass")) return debugger(TRUE, argv[2]);
    printf("usage: attach.exe <continue|pass> <target.exe>\n");
    return 1;
}
