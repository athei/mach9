/* The program bundle-wine.sh builds for x86_64 and i686 to check that a child
 * that a program starts with an environment block of its own gets the x87
 * precision the program has: that of compatdb's built-in rule for reduced
 * precision, or of a FEX_X87REDUCEDPRECISION exported where Wine started.
 * Wine starts such a child with the program's unix environment and the block
 * it was given, and FEX reads its options from the block.
 *
 * childenv.exe prints its line, starts "childenv.exe child" with a copy of its
 * own block without FEX_X87REDUCEDPRECISION, waits for it with a time limit
 * and exits with its exit code. Each prints
 *
 *   wine-build smoke test: <arch> childenv <parent|child>:
 *       FEX_X87REDUCEDPRECISION=<value> x87 <full|reduced>
 *
 * on one line, the value being "(unset)" when its block has none. With the
 * precision control at 64 bits, x87 code keeps 1 + 2^-60 apart from 1 at FEX's
 * full precision, while its reduced precision computes in doubles. */

#include <stdio.h>
#include <string.h>
#include <wchar.h>
#include <windows.h>

#if defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

#define VAR "FEX_X87REDUCEDPRECISION"
#define CHILD_WAIT_MS 2000

static const double one = 1.0;
static const double tiny = 0x1p-60;

/* "full" when x87 code computes at 64-bit precision, "reduced" when it does
 * not. */
static const char *x87_precision(void)
{
    unsigned short cw, extended;
    double r;

    __asm__ volatile("fnstcw %0" : "=m"(cw));
    extended = cw | 0x300;
    __asm__ volatile("fldcw %1\n\t"
                     "fldl %2\n\t"
                     "faddl %3\n\t"
                     "fsubl %2\n\t"
                     "fstpl %0\n\t"
                     "fldcw %4"
                     : "=m"(r)
                     : "m"(extended), "m"(one), "m"(tiny), "m"(cw));
    return r != 0.0 ? "full" : "reduced";
}

static void report(const char *who)
{
    char value[64];
    DWORD len = GetEnvironmentVariableA(VAR, value, sizeof(value));

    if (!len && GetLastError() == ERROR_ENVVAR_NOT_FOUND)
        strcpy(value, "(unset)");
    else if (len >= sizeof(value))
        strcpy(value, "(too long)");
    printf("wine-build smoke test: %s childenv %s: %s=%s x87 %s\n", ARCH, who, VAR, value,
           x87_precision());
    fflush(stdout);
}

/* A copy of this process's environment block without VAR, or NULL. */
static WCHAR *block_without_var(void)
{
    static const WCHAR prefix[] = L"" VAR "=";
    WCHAR *env = GetEnvironmentStringsW(), *copy, *out;
    const WCHAR *entry;
    size_t size = 0;

    if (!env) return NULL;
    for (entry = env; *entry; entry += wcslen(entry) + 1) size += wcslen(entry) + 1;
    if (!(copy = HeapAlloc(GetProcessHeap(), 0, (size + 1) * sizeof(WCHAR))))
    {
        FreeEnvironmentStringsW(env);
        return NULL;
    }
    out = copy;
    for (entry = env; *entry; entry += wcslen(entry) + 1)
    {
        if (!_wcsnicmp(entry, prefix, wcslen(prefix))) continue;
        wcscpy(out, entry);
        out += wcslen(entry) + 1;
    }
    *out = 0;
    FreeEnvironmentStringsW(env);
    return copy;
}

int main(int argc, char **argv)
{
    WCHAR exe[MAX_PATH], cmd[MAX_PATH + 16], *block;
    STARTUPINFOW si = { sizeof(si) };
    PROCESS_INFORMATION pi;
    DWORD code = 1;
    HANDLE out;

    if (argc == 2 && !strcmp(argv[1], "child"))
    {
        report("child");
        return 0;
    }
    report("parent");
    if (!GetModuleFileNameW(NULL, exe, MAX_PATH) || !(block = block_without_var())
        || !DuplicateHandle(GetCurrentProcess(), GetStdHandle(STD_OUTPUT_HANDLE),
                            GetCurrentProcess(), &out, 0, TRUE, DUPLICATE_SAME_ACCESS))
    {
        printf("wine-build smoke test: %s childenv parent failed: error %lu\n", ARCH, GetLastError());
        return 1;
    }
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = si.hStdError = out;
    wcscpy(cmd, L"\"");
    wcscat(cmd, exe);
    wcscat(cmd, L"\" child");
    if (!CreateProcessW(exe, cmd, NULL, NULL, TRUE, CREATE_UNICODE_ENVIRONMENT, block, NULL, &si,
                        &pi))
    {
        printf("wine-build smoke test: %s childenv parent failed: CreateProcess error %lu\n", ARCH,
               GetLastError());
        return 1;
    }
    if (WaitForSingleObject(pi.hProcess, CHILD_WAIT_MS) != WAIT_OBJECT_0)
    {
        printf("wine-build smoke test: %s childenv parent failed: the child did not end within %d ms\n",
               ARCH, CHILD_WAIT_MS);
        TerminateProcess(pi.hProcess, 1);
        return 1;
    }
    GetExitCodeProcess(pi.hProcess, &code);
    return code;
}
