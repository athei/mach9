/* A real Windows parent and child running the same PE image. Only the
 * child's explicit Windows environment block contains SteamAppId. */
#define WIN32_LEAN_AND_MEAN
#include <stddef.h>
#include <stdio.h>
#include <wchar.h>
#include <windows.h>
#include <wine/server.h>

#define STEAM_ID L"4294967294"
#define WAIT_MS 45000

static int fail(const char *message)
{
    fprintf(stderr, "named-loader-child: %s (GetLastError = %lu)\n", message, GetLastError());
    return 1;
}

static int native_pid(void)
{
    __typeof__(&wine_server_call) call;
    int pid = -1;

    /* Windows PIDs aren't host PIDs. Ask the matching Wine build's server
     * for the native PID of this thread, not ps for a matching command line. */
    call = (__typeof__(call))(void *)GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "wine_server_call");
    if (!call) return -1;
    SERVER_START_REQ(get_thread_times)
    {
        req->handle = wine_server_obj_handle(GetCurrentThread());
        if (!call(&__req)) pid = reply->unix_pid;
    }
    SERVER_END_REQ;
    return pid;
}

static int child(void)
{
    WCHAR id[32];
    ULONGLONG start;
    int pid;

    if (GetEnvironmentVariableW(L"SteamAppId", id, ARRAY_SIZE(id)) != ARRAY_SIZE(STEAM_ID) - 1 || wcscmp(id, STEAM_ID))
        return fail("the child didn't receive SteamAppId");
    if (GetFileAttributesW(L"hold-child") == INVALID_FILE_ATTRIBUTES) return fail("couldn't find hold-child");
    if ((pid = native_pid()) <= 0) return fail("couldn't obtain the child's native PID");
    printf("named-loader-child-ready %lu %d\n", GetCurrentProcessId(), pid);
    fflush(stdout);

    /* The host removes this file after examining both processes (or when
     * cleaning up on failure). Bound the wait even if the host dies. */
    start = GetTickCount64();
    do
    {
        if (GetFileAttributesW(L"hold-child") == INVALID_FILE_ATTRIBUTES)
        {
            DWORD error = GetLastError();
            if (error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND) return 0;
            return fail("couldn't check hold-child");
        }
        Sleep(50);
    } while (GetTickCount64() - start < WAIT_MS);
    return fail("timed out waiting for hold-child to be removed");
}

static int parent(void)
{
    WCHAR exe[MAX_PATH], cmd[MAX_PATH + 16], id[32];
    WCHAR env[] = L"SteamAppId=" STEAM_ID L"\0";
    STARTUPINFOW si = { .cb = sizeof(si), .dwFlags = STARTF_USESTDHANDLES };
    PROCESS_INFORMATION pi;
    DWORD len, code;
    int result = 0;

    SetLastError(ERROR_SUCCESS);
    if (GetEnvironmentVariableW(L"SteamAppId", id, ARRAY_SIZE(id)) || GetLastError() != ERROR_ENVVAR_NOT_FOUND)
        return fail("SteamAppId leaked into the parent");
    len = GetModuleFileNameW(NULL, exe, ARRAY_SIZE(exe));
    if (!len || len >= ARRAY_SIZE(exe)) return fail("couldn't get the executable path");
    swprintf(cmd, ARRAY_SIZE(cmd), L"\"%ls\" child", exe);
    si.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
    si.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
    si.hStdError = GetStdHandle(STD_ERROR_HANDLE);
    if (!CreateProcessW(exe, cmd, NULL, NULL, TRUE, CREATE_UNICODE_ENVIRONMENT, env, NULL, &si, &pi))
        return fail("CreateProcessW failed");
    CloseHandle(pi.hThread);
    printf("named-loader-child-created %lu\n", pi.dwProcessId);
    fflush(stdout);
    if (WaitForSingleObject(pi.hProcess, WAIT_MS) != WAIT_OBJECT_0)
    {
        result = fail("the child didn't exit");
        TerminateProcess(pi.hProcess, 1);
    }
    else if (!GetExitCodeProcess(pi.hProcess, &code) || code) result = fail("the child failed");
    CloseHandle(pi.hProcess);
    return result;
}

int wmain(int argc, WCHAR **argv)
{
    if (argc == 1) return parent();
    if (argc == 2 && !wcscmp(argv[1], L"child")) return child();
    return fail("unexpected arguments");
}
