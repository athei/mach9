/* The program bundle-wine.sh builds for each PE architecture and runs under
 * the bundled wine to check system calls whose result on an arm64 host has to
 * match Windows although the host works differently:
 *
 *   selectors  every LDT selector below 0x100 through
 *              RtlWow64GetThreadSelectorEntry() (aarch64, x86_64) or
 *              NtQueryInformationThread(ThreadDescriptorTableEntry) (i686),
 *              and the ThreadDescriptorTableEntry query with a wrong length.
 *              The host has no LDT; Windows answers STATUS_UNSUCCESSFUL or
 *              STATUS_NO_LDT, and STATUS_INFO_LENGTH_MISMATCH, and leaves the
 *              returned length alone, as Wine's ntdll:wow64 test_selectors
 *              checks. ntdll answered STATUS_NOT_IMPLEMENTED (mach9 #198).
 *   own-write  WriteProcessMemory(GetCurrentProcess()) into a page that is
 *              writable and executable and whose native code ran after the
 *              protect, three times. On aarch64 it is code of its own, changed
 *              by each write and run after it. On x86_64 it is the ARM64EC code
 *              of ntdll's RtlComputeCrc32, which FEX runs natively, written
 *              back unchanged and run after each write. macOS maps such a page
 *              either writable or executable, and Wine flips it on a fault, so
 *              the wineserver's write found it executable and failed with
 *              ERROR_PARTIAL_COPY (mach9 #196).
 *   own-write-handle  the same with a handle OpenProcess() gives for this
 *              process, writing the code unchanged on aarch64 too, and then a
 *              write through a handle without PROCESS_VM_WRITE, which has to
 *              fail with ERROR_ACCESS_DENIED.
 *   own-read   ReadFile() into such a page, READ_ROUNDS times, while another
 *              thread runs the page's code: on aarch64 into the page's data
 *              after its code, on x86_64 the code's own bytes. NtReadFile()
 *              makes the buffer writable with a write of its own first, so the
 *              kernel's write found the page executable, and the read failed,
 *              only when the other thread flipped it back in between (mach9
 *              #196).
 *
 * An i686 program reaches no native code through its pointers, so only
 * selectors is built there.
 *
 * The cases run in order, or just those named on the command line. Each
 * prints its name first. A failed case prints why and ends the process with
 * 1; the exit code is 0 only when every case passed. */

#include <stdio.h>
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

#define ThreadDescriptorTableEntry 6
#define STATUS_UNSUCCESSFUL ((LONG)0xc0000001)
#define STATUS_INFO_LENGTH_MISMATCH ((LONG)0xc0000004)
#define STATUS_NO_LDT ((LONG)0xc0000117)
#define WRITE_ROUNDS 3
#define READ_ROUNDS 2000

static const char *current = "startup";

#define FAIL(...) do { \
        printf("wine-build smoke test: %s %s failed, line %d: ", ARCH, current, __LINE__); \
        printf(__VA_ARGS__); \
        printf("\n"); \
        TerminateProcess(GetCurrentProcess(), 1); \
    } while (0)

/* THREAD_DESCRIPTOR_INFORMATION, with the LDT_ENTRY as two words */
struct selector_info
{
    DWORD selector;
    DWORD entry[2];
};

static LONG (WINAPI *pNtQueryInformationThread)(HANDLE, int, void *, ULONG, ULONG *);
#ifndef __i386__
static LONG (WINAPI *pRtlWow64GetThreadSelectorEntry)(HANDLE, struct selector_info *, ULONG, ULONG *);
#endif

static void *ntdll_proc(const char *name)
{
    void *proc = (void *)GetProcAddress(GetModuleHandleA("ntdll.dll"), name);

    if (!proc) FAIL("ntdll has no %s", name);
    return proc;
}

static LONG get_entry(struct selector_info *info, ULONG size, ULONG *retlen)
{
#ifdef __i386__
    return pNtQueryInformationThread(GetCurrentThread(), ThreadDescriptorTableEntry, info, size, retlen);
#else
    return pRtlWow64GetThreadSelectorEntry(GetCurrentThread(), info, size, retlen);
#endif
}

static void selectors(void)
{
    struct selector_info info;
    ULONG retlen;
    DWORD sel;
    LONG status;

    pNtQueryInformationThread = ntdll_proc("NtQueryInformationThread");
#ifndef __i386__
    pRtlWow64GetThreadSelectorEntry = ntdll_proc("RtlWow64GetThreadSelectorEntry");
#endif
    for (sel = 0; sel < 0x100; sel++)
    {
        if (!(sel & 4)) continue;
        memset(&info, 0, sizeof(info));
        info.selector = sel;
        retlen = 0xdeadbeef;
        status = get_entry(&info, sizeof(info), &retlen);
        if (status != STATUS_UNSUCCESSFUL && status != STATUS_NO_LDT)
            FAIL("selector %#lx: status %#lx, wanted %#lx or %#lx", sel, status, STATUS_UNSUCCESSFUL,
                 STATUS_NO_LDT);
        if (retlen != 0xdeadbeef) FAIL("selector %#lx: returned length set to %lu", sel, retlen);
    }

    info.selector = 0x0f;
    retlen = 0xdeadbeef;
    status = pNtQueryInformationThread(GetCurrentThread(), ThreadDescriptorTableEntry, &info, sizeof(info) - 1,
                                       &retlen);
    if (status != STATUS_INFO_LENGTH_MISMATCH || retlen != 0xdeadbeef)
        FAIL("length %u: status %#lx, returned length %lu, wanted %#lx and no length", (unsigned int)sizeof(info) - 1,
             status, retlen, STATUS_INFO_LENGTH_MISMATCH);
}

#ifndef __i386__
static void write_own(HANDLE process, void *addr, const void *data, SIZE_T size)
{
    SIZE_T written = 0;

    if (!WriteProcessMemory(process, addr, data, size, &written))
        FAIL("WriteProcessMemory to %p: error %lu", addr, GetLastError());
    if (written != size) FAIL("WriteProcessMemory to %p wrote %lu of %lu bytes", addr, (ULONG)written, (ULONG)size);
}

static void run_native(void);
static volatile LONG stop_runner;

/* runs native code on the page under test until told to stop */
static DWORD WINAPI runner(void *arg)
{
    while (!stop_runner) run_native();
    return 0;
}

/* READ_ROUNDS ReadFile() calls of the data, which a temporary file holds, into dst, while another
 * thread runs code on dst's page */
static void read_own(void *dst, const void *data, DWORD size)
{
    char dir[MAX_PATH], name[MAX_PATH];
    HANDLE file, thread;
    DWORD done = 0;
    int round;

    if (!GetTempPathA(sizeof(dir), dir) || !GetTempFileNameA(dir, "ntd", 0, name))
        FAIL("temporary file name: error %lu", GetLastError());
    file = CreateFileA(name, GENERIC_READ | GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                       FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE, NULL);
    if (file == INVALID_HANDLE_VALUE) FAIL("CreateFile %s: error %lu", name, GetLastError());
    if (!WriteFile(file, data, size, &done, NULL) || done != size) FAIL("WriteFile: error %lu", GetLastError());
    stop_runner = 0;
    if (!(thread = CreateThread(NULL, 0, runner, NULL, 0, NULL))) FAIL("CreateThread: error %lu", GetLastError());
    for (round = 1; round <= READ_ROUNDS; round++)
    {
        SetFilePointer(file, 0, NULL, FILE_BEGIN);
        done = 0;
        if (!ReadFile(file, dst, size, &done, NULL))
            FAIL("round %d: ReadFile into %p: error %lu", round, dst, GetLastError());
        if (done != size) FAIL("round %d: ReadFile into %p read %lu of %lu bytes", round, dst, done, size);
        if (memcmp(dst, data, size)) FAIL("round %d: ReadFile into %p: wrong data", round, dst);
    }
    InterlockedExchange(&stop_runner, 1);
    WaitForSingleObject(thread, INFINITE);
    CloseHandle(thread);
    CloseHandle(file);
}

static HANDLE open_own(void)
{
    HANDLE process = OpenProcess(PROCESS_ALL_ACCESS, FALSE, GetCurrentProcessId());

    if (!process) FAIL("OpenProcess: error %lu", GetLastError());
    return process;
}

/* a handle to this process without PROCESS_VM_WRITE may not write, also into a flipped page */
static void write_denied(void *addr, const void *data, SIZE_T size)
{
    HANDLE process = OpenProcess(PROCESS_VM_READ | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION, FALSE,
                                 GetCurrentProcessId());
    SIZE_T written = 0;

    if (!process) FAIL("OpenProcess: error %lu", GetLastError());
    if (WriteProcessMemory(process, addr, data, size, &written))
        FAIL("WriteProcessMemory to %p through a handle without PROCESS_VM_WRITE succeeded", addr);
    if (GetLastError() != ERROR_ACCESS_DENIED)
        FAIL("WriteProcessMemory to %p through a handle without PROCESS_VM_WRITE: error %lu, wanted %lu", addr,
             GetLastError(), (DWORD)ERROR_ACCESS_DENIED);
    CloseHandle(process);
}
#endif

#if defined(__aarch64__) && !defined(__arm64ec__)
/* mov w0, #value; ret */
static void put_code(UINT32 *code, int value)
{
    code[0] = 0x52800000 | (value << 5);
    code[1] = 0xd65f03c0;
}

static void run_code(UINT32 *page, int want)
{
    int got = ((int (*)(void))page)();

    if (got != want) FAIL("code at %p returned %d, wanted %d", page, got, want);
}

/* A writable and executable page with code that returns 0, run once, which makes Wine map it
 * executable */
static UINT32 *new_code_page(void)
{
    UINT32 *page = VirtualAlloc(NULL, 0x1000, MEM_RESERVE | MEM_COMMIT, PAGE_EXECUTE_READWRITE);

    if (!page) FAIL("VirtualAlloc: error %lu", GetLastError());
    put_code(page, 0);
    FlushInstructionCache(GetCurrentProcess(), page, 8);
    run_code(page, 0);
    return page;
}

static void own_write(void)
{
    UINT32 *page = new_code_page(), code[2];
    int round;

    for (round = 1; round <= WRITE_ROUNDS; round++)
    {
        put_code(code, round);
        write_own(GetCurrentProcess(), page, code, sizeof(code));
        /* runs the new code, which makes Wine map the page executable again */
        run_code(page, round);
    }
    VirtualFree(page, 0, MEM_RELEASE);
}

/* kernelbase flushes the instruction cache through the handle it got, which ntdll's
 * NtFlushInstructionCache() does only for the pseudo handle, so these writes leave the code as
 * it is */
static void own_write_handle(void)
{
    UINT32 *page = new_code_page(), code[2];
    HANDLE process = open_own();
    int round;

    put_code(code, 0);
    for (round = 1; round <= WRITE_ROUNDS; round++)
    {
        write_own(process, page, code, sizeof(code));
        run_code(page, 0);
    }
    write_denied(page, code, sizeof(code));
    CloseHandle(process);
    VirtualFree(page, 0, MEM_RELEASE);
}

static UINT32 *read_page;

static void run_native(void)
{
    run_code(read_page, 0);
}

static void own_read(void)
{
    static const char data[] = "wine-build smoke test";

    read_page = new_code_page();
    read_own((BYTE *)read_page + 0x800, data, sizeof(data));
    run_code(read_page, 0);
    VirtualFree(read_page, 0, MEM_RELEASE);
}
#elif defined(__x86_64__)
/* The thunk lld puts in front of an exported ARM64EC function for x86_64 callers, which
 * GetProcAddress() returns: mov rax, rsp; mov [rax+0x20], rbx; push rbp; pop rbp; jmp rel32 */
static const BYTE ec_thunk[] = { 0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x55, 0x5d, 0xe9 };
static const BYTE crc_data[] = "wine-build smoke test";
static DWORD (WINAPI *crc)(DWORD, const BYTE *, LONG);
static BYTE *crc_code, crc_bytes[16];
static DWORD crc_want;

/* RtlComputeCrc32's ARM64EC code, which FEX runs natively when x86_64 code calls it */
static void find_crc(void)
{
    BOOLEAN (WINAPI *is_ec)(const void *) = ntdll_proc("RtlIsEcCode");
    BYTE *thunk = ntdll_proc("RtlComputeCrc32");
    INT32 rel;

    if (crc_code) return;
    if (memcmp(thunk, ec_thunk, sizeof(ec_thunk))) FAIL("RtlComputeCrc32 at %p is not an export thunk", thunk);
    memcpy(&rel, thunk + sizeof(ec_thunk), sizeof(rel));
    crc_code = thunk + sizeof(ec_thunk) + sizeof(rel) + rel;
    if (!is_ec(crc_code)) FAIL("RtlComputeCrc32's code at %p is not ARM64EC code", crc_code);
    crc = (void *)thunk;
    memcpy(crc_bytes, crc_code, sizeof(crc_bytes));
    crc_want = crc(0, crc_data, sizeof(crc_data));
}

static void run_native(void)
{
    DWORD got = crc(0, crc_data, sizeof(crc_data));

    if (got != crc_want) FAIL("RtlComputeCrc32 returned %#lx, wanted %#lx", got, crc_want);
}

/* makes the code's page writable and executable and runs the code, which makes Wine map the page
 * executable; returns the old protection */
static DWORD protect_crc(void)
{
    DWORD old;

    find_crc();
    if (!VirtualProtect(crc_code, sizeof(crc_bytes), PAGE_EXECUTE_READWRITE, &old))
        FAIL("VirtualProtect: error %lu", GetLastError());
    run_native();
    return old;
}

static void restore_crc(DWORD old)
{
    DWORD prot;

    if (!VirtualProtect(crc_code, sizeof(crc_bytes), old, &prot)) FAIL("VirtualProtect back: error %lu", GetLastError());
    if (memcmp(crc_code, crc_bytes, sizeof(crc_bytes))) FAIL("RtlComputeCrc32's code changed");
    run_native();
}

/* writes the code back unchanged and runs it after each write */
static void write_rounds(HANDLE process, BOOL check_denied)
{
    DWORD old = protect_crc();
    int round;

    for (round = 1; round <= WRITE_ROUNDS; round++)
    {
        write_own(process, crc_code, crc_bytes, sizeof(crc_bytes));
        run_native();
    }
    if (check_denied) write_denied(crc_code, crc_bytes, sizeof(crc_bytes));
    restore_crc(old);
}

static void own_write(void)
{
    write_rounds(GetCurrentProcess(), FALSE);
}

static void own_write_handle(void)
{
    HANDLE process = open_own();

    write_rounds(process, TRUE);
    CloseHandle(process);
}

/* reads the code's own bytes into it */
static void own_read(void)
{
    DWORD old = protect_crc();

    read_own(crc_code, crc_bytes, sizeof(crc_bytes));
    restore_crc(old);
}
#endif

static const struct
{
    const char *name;
    void (*proc)(void);
} cases[] =
{
    { "selectors", selectors },
#ifndef __i386__
    { "own-write", own_write },
    { "own-write-handle", own_write_handle },
    { "own-read", own_read },
#endif
};

int main(int argc, char **argv)
{
    unsigned int i;
    int arg, ran = 0;

    setvbuf(stdout, NULL, _IONBF, 0);
    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        for (arg = 1; arg < argc; arg++)
            if (!strcmp(argv[arg], cases[i].name)) break;
        if (argc > 1 && arg == argc) continue;
        current = cases[i].name;
        printf("wine-build smoke test: %s case %s\n", ARCH, current);
        cases[i].proc();
        ran++;
    }
    if (!ran)
    {
        printf("wine-build smoke test: %s no such case\n", ARCH);
        return 1;
    }
    printf("wine-build smoke test: %s ntdll cases passed\n", ARCH);
    return 0;
}
