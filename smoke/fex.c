/* The program bundle-wine.sh builds for each x86 PE architecture and runs under
 * FEX, once in its default mode and once with FEX_SMCCHECKS=full. Its cases run
 * code in reserved memory and on guard pages, writes to compiled pages,
 * instructions FEX cannot decode or fetch whole, and pages taken away or changed
 * while code runs from them. FEX has hung on or reported most of these wrongly;
 * the others guard behaviour that already worked.
 *
 * The cases run in order in one process, or just those named on the command
 * line. Each prints its name first. A failed case prints why and ends the
 * process with 1; the usual failure is a hang, and the last line then names the
 * case. There is no watchdog thread, since a stalled JIT stops that as well, so
 * the caller's time limit ends a hang. The exit code is 0 only when every case
 * passed.
 *
 * Do not add a case that passes the old protection or another out-param of an
 * Nt call pointing into a compiled RWX page, or that changes the memory of a
 * process that is compiling from another process: FEX still hangs on those. Do
 * not use TerminateThread on a thread that has just started, which can leave the
 * guest's loader lock held. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>

#if defined(__x86_64__)
#define ARCH "x86_64"
#define CTX_PC(c) ((c)->Rip)
#define CTX_SP(c) ((c)->Rsp)
#define CTX_CX(c) ((c)->Rcx)
#elif defined(__i386__)
#define ARCH "i686"
#define CTX_PC(c) ((c)->Eip)
#define CTX_SP(c) ((c)->Esp)
#define CTX_CX(c) ((c)->Ecx)
#else
#error "unknown architecture"
#endif

#define PAGE 0x1000
#define SLED_PAGES 16
#define SLED_SIZE (SLED_PAGES * PAGE)
#define AV EXCEPTION_ACCESS_VIOLATION
#define UD EXCEPTION_ILLEGAL_INSTRUCTION
#define FETCH EXCEPTION_EXECUTE_FAULT

/* How long the races run. Every cycle of the first takes about 25 ms under
 * FEX_SMCCHECKS=full. */
#define DECOMMIT_CYCLES 100
#define STALE_CALLS 100
#define RWX_ROUNDS 3000
#define RWX_ROUNDS_FULL 200
#define GUARD_THREADS 2
#define GUARD_ROUNDS 100

static const char *current = "startup";

#define FAIL(...) do { \
        printf("wine-build smoke test: %s %s failed, line %d: ", ARCH, current, __LINE__); \
        printf(__VA_ARGS__); \
        printf("\n"); \
        TerminateProcess(GetCurrentProcess(), 1); \
    } while (0)

/* A fault with a pc in the code under test returns to its caller, as that code
 * is entered with a call and leaves the stack alone. Any other fault ends the
 * case. */
static volatile ULONG_PTR code_lo, code_hi;
static volatile LONG faults, fetch_faults, guards;
static volatile DWORD fault_code, fault_params;
static volatile ULONG_PTR fault_info0, fault_info1, fault_pc, fault_cx;
static volatile ULONG_PTR guard_info0, guard_info1, guard_pc;

static LONG CALLBACK handler(EXCEPTION_POINTERS *e)
{
    EXCEPTION_RECORD *r = e->ExceptionRecord;
    CONTEXT *c = e->ContextRecord;
    ULONG_PTR pc = (ULONG_PTR)CTX_PC(c);
    ULONG_PTR *sp = (ULONG_PTR *)(ULONG_PTR)CTX_SP(c);

    if (r->ExceptionCode == EXCEPTION_GUARD_PAGE)
    {
        InterlockedIncrement(&guards);
        guard_info0 = r->ExceptionInformation[0];
        guard_info1 = r->ExceptionInformation[1];
        guard_pc = pc;
        return EXCEPTION_CONTINUE_EXECUTION;
    }
    if (r->ExceptionCode != AV && r->ExceptionCode != UD) return EXCEPTION_CONTINUE_SEARCH;
    if (pc < code_lo || pc >= code_hi)
        FAIL("exception %#lx at %p outside the code, [0]=%p [1]=%p", r->ExceptionCode, (void *)pc,
             (void *)r->ExceptionInformation[0], (void *)r->ExceptionInformation[1]);

    InterlockedIncrement(&faults);
    fault_code = r->ExceptionCode;
    fault_params = r->NumberParameters;
    fault_info0 = r->ExceptionInformation[0];
    fault_info1 = r->ExceptionInformation[1];
    fault_pc = pc;
    fault_cx = (ULONG_PTR)CTX_CX(c);
    /* After the fields, which decommit_race reads once it sees this count */
    if (r->ExceptionCode == AV && r->ExceptionInformation[0] == FETCH) InterlockedIncrement(&fetch_faults);
    CTX_PC(c) = *sp;
    CTX_SP(c) += sizeof(*sp);
    return EXCEPTION_CONTINUE_EXECUTION;
}

static BYTE *new_pages(SIZE_T size, DWORD prot)
{
    BYTE *p = VirtualAlloc(NULL, size, MEM_RESERVE | MEM_COMMIT, prot);

    if (!p) FAIL("VirtualAlloc: %lu", GetLastError());
    return p;
}

static void protect(void *p, SIZE_T size, DWORD prot)
{
    DWORD old;

    if (!VirtualProtect(p, size, prot, &old)) FAIL("VirtualProtect: %lu", GetLastError());
}

static void set_code(void *p, SIZE_T size)
{
    code_lo = (ULONG_PTR)p;
    code_hi = (ULONG_PTR)p + size;
}

static void spin(unsigned int n)
{
    volatile unsigned int i;

    for (i = 0; i < n; i++);
}

/* mov eax, value; ret */
static void put_code(BYTE *p, int value)
{
    p[0] = 0xb8;
    memcpy(p + 1, &value, 4);
    p[5] = 0xc3;
}

static void check_fault(DWORD code, const void *pc, ULONG_PTR info0, const void *info1)
{
    if (faults != 1 || fault_code != code || fault_pc != (ULONG_PTR)pc)
        FAIL("%ld faults, code %#lx pc %p, wanted one %#lx at %p", faults, fault_code,
             (void *)fault_pc, code, pc);
    if (code == AV && (fault_params != 2 || fault_info0 != info0 || fault_info1 != (ULONG_PTR)info1))
        FAIL("%lu parameters, [0]=%p [1]=%p, wanted 2, %p %p", fault_params, (void *)fault_info0,
             (void *)fault_info1, (void *)info0, info1);
}

static void check_guard(const char *what, void *addr, int want, int got)
{
    if (guards != 1 || guard_info0 != FETCH || guard_info1 != (ULONG_PTR)addr
            || guard_pc != (ULONG_PTR)addr)
        FAIL("%s: %ld guard exceptions, [0]=%p [1]=%p pc %p, wanted one at %p", what, guards,
             (void *)guard_info0, (void *)guard_info1, (void *)guard_pc, addr);
    if (got != want) FAIL("%s: returned %d, wanted %d", what, got, want);
    if (faults) FAIL("%s: %ld faults, [0]=%p [1]=%p", what, faults, (void *)fault_info0,
                     (void *)fault_info1);
}

/* Falls through the end of a committed page into a reserved one */
static void reserved_fall_through(void)
{
    BYTE *base = VirtualAlloc(NULL, 2 * PAGE, MEM_RESERVE, PAGE_EXECUTE_READWRITE);
    void (*fn)(void) = (void *)base;

    if (!base || !VirtualAlloc(base, PAGE, MEM_COMMIT, PAGE_EXECUTE_READWRITE))
        FAIL("VirtualAlloc: %lu", GetLastError());
    memset(base, 0x90, PAGE);
    set_code(base, 2 * PAGE);
    fn();
    check_fault(AV, base + PAGE, FETCH, base + PAGE);
}

static void reserved_call(void)
{
    BYTE *base = VirtualAlloc(NULL, 2 * PAGE, MEM_RESERVE, PAGE_EXECUTE_READWRITE);
    void (*fn)(void) = (void *)base;

    if (!base) FAIL("VirtualAlloc: %lu", GetLastError());
    set_code(base, 2 * PAGE);
    fn();
    check_fault(AV, base, FETCH, base);
}

/* An executable guard page holding code in private memory */
static void guard_call(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)page;
    int r;

    put_code(page, 7);
    set_code(page, PAGE);
    protect(page, PAGE, PAGE_EXECUTE_READ | PAGE_GUARD);
    r = fn();
    check_guard("first call", page, 7, r);
    r = fn();
    check_guard("second call", page, 7, r);
}

/* The same in a view of a section */
static void guard_view(void)
{
    HANDLE section = CreateFileMappingA(INVALID_HANDLE_VALUE, NULL, PAGE_EXECUTE_READWRITE, 0, 0x10000, NULL);
    BYTE *rw = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 0x10000);
    BYTE *rx = MapViewOfFile(section, FILE_MAP_READ | FILE_MAP_EXECUTE, 0, 0, 0x10000);
    int (*fn)(void) = (void *)rx;
    int r;

    if (!rw || !rx) FAIL("MapViewOfFile: %lu", GetLastError());
    put_code(rw, 7);
    set_code(rx, 0x10000);
    protect(rx, PAGE, PAGE_EXECUTE_READ | PAGE_GUARD);
    r = fn();
    check_guard("first call", rx, 7, r);
    r = fn();
    check_guard("second call", rx, 7, r);
}

/* Falls through the end of a page into a guard page */
static void guard_fall_through(void)
{
    BYTE *base = new_pages(2 * PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)base;
    int r;

    memset(base, 0x90, PAGE);
    put_code(base + PAGE, 8);
    set_code(base, 2 * PAGE);
    protect(base, PAGE, PAGE_EXECUTE_READ);
    protect(base + PAGE, PAGE, PAGE_EXECUTE_READ | PAGE_GUARD);
    r = fn();
    check_guard("call", base + PAGE, 8, r);
}

/* RWX and guard, then a write to the code once the guard is gone */
static void guard_smc(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)page;
    int r;

    put_code(page, 9);
    set_code(page, PAGE);
    protect(page, PAGE, PAGE_EXECUTE_READWRITE | PAGE_GUARD);
    r = fn();
    check_guard("first call", page, 9, r);
    put_code(page, 10);
    r = fn();
    check_guard("after the write", page, 10, r);
}

/* A data read uses up the guard, and the code then just runs */
static void guard_read_then_call(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)page;
    BYTE value;
    int r;

    put_code(page, 19);
    set_code(page, PAGE);
    protect(page, PAGE, PAGE_EXECUTE_READ | PAGE_GUARD);
    value = *(volatile BYTE *)page;
    if (guards != 1 || guard_info0 != 0 || value != 0xb8)
        FAIL("read: %ld guard exceptions, [0]=%p value %#x", guards, (void *)guard_info0, value);
    r = fn();
    if (r != 19 || guards != 1 || faults)
        FAIL("call: returned %d, %ld guard exceptions, %ld faults", r, guards, faults);
}

/* Each protect arms the guard again, over a page that has compiled code */
static void guard_rearm(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)page;
    int i, r;

    put_code(page, 20);
    set_code(page, PAGE);
    for (i = 1; i <= 100; i++)
    {
        protect(page, PAGE, PAGE_EXECUTE_READ | PAGE_GUARD);
        r = fn();
        if (r != 20 || guards != i || guard_info0 != FETCH || faults)
            FAIL("round %d: returned %d, %ld guard exceptions, [0]=%p, %ld faults", i, r, guards,
                 (void *)guard_info0, faults);
        r = fn();
        if (r != 20 || guards != i)
            FAIL("round %d, second call: returned %d, %ld guard exceptions", i, r, guards);
    }
}

/* A guard page that is not executable: the guard's exception, then the fault */
static void guard_noexec(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    void (*fn)(void) = (void *)page;

    put_code(page, 21);
    set_code(page, PAGE);
    protect(page, PAGE, PAGE_READWRITE | PAGE_GUARD);
    fn();
    if (guards != 1 || guard_info0 != FETCH || guard_info1 != (ULONG_PTR)page)
        FAIL("%ld guard exceptions, [0]=%p", guards, (void *)guard_info0);
    check_fault(AV, page, FETCH, page);
}

/* An instruction from an executable page into a guard page. [1] is the first
 * byte on the guard page, the pc the instruction, which cannot be fetched
 * whole. */
static void guard_straddle(DWORD first_prot, DWORD second_prot)
{
    BYTE *base = new_pages(2 * PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)(base + PAGE - 8);
    BYTE *insn = base + PAGE - 2;
    int r;

    memset(base, 0x90, PAGE);
    insn[0] = 0xb8; /* mov eax, imm32 */
    insn[1] = 0x27;
    memset(base + PAGE, 0, 3);
    base[PAGE + 3] = 0xc3;
    set_code(base, 2 * PAGE);
    protect(base, PAGE, first_prot);
    protect(base + PAGE, PAGE, second_prot | PAGE_GUARD);

    r = fn();
    if (guards != 1 || guard_info0 != FETCH || guard_info1 != (ULONG_PTR)(base + PAGE)
            || guard_pc != (ULONG_PTR)insn)
        FAIL("%ld guard exceptions, [0]=%p [1]=%p pc %p, wanted one at %p pc %p", guards,
             (void *)guard_info0, (void *)guard_info1, (void *)guard_pc, base + PAGE, insn);
    if (r != 0x27 || faults) FAIL("returned %#x, %ld faults", r, faults);
    r = fn();
    if (r != 0x27 || guards != 1 || faults)
        FAIL("second call returned %#x, %ld guard exceptions, %ld faults", r, guards, faults);

    if (first_prot == PAGE_EXECUTE_READWRITE)
    {
        insn[1] = 0x28;
        r = fn();
        if (r != 0x28) FAIL("a write to the first page: returned %#x", r);
    }
    if (second_prot == PAGE_EXECUTE_READWRITE)
    {
        base[PAGE] = 0x01;
        r = fn();
        if ((r & 0xff00) != 0x0100) FAIL("a write to the second page: returned %#x", r);
    }
    if (guards != 1 || faults) FAIL("after the writes: %ld guard exceptions, %ld faults", guards, faults);
}

static void guard_straddle_rx_rx(void)
{
    guard_straddle(PAGE_EXECUTE_READ, PAGE_EXECUTE_READ);
}

static void guard_straddle_rwx_rx(void)
{
    guard_straddle(PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_READ);
}

static void guard_straddle_rwx_rwx(void)
{
    guard_straddle(PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_READWRITE);
}

/* Writes that fault on a page FEX protected, with no fault for the guest */
static void trapped_writes(void)
{
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)page;
    volatile LONG *target = (volatile LONG *)(page + 0x800);
    static const BYTE new_code[6] = {0xb8, 0x02, 0x00, 0x00, 0x00, 0xc3};
    LONG expected = 1, desired = 2;
    const BYTE *src = new_code;
    BYTE *dst = page;
    size_t count = sizeof(new_code);
    int i;

    put_code(page, 1);
    *target = 5;
    if (fn() != 1) FAIL("first call");
    for (i = 0; i < 100; i++)
    {
        asm volatile("lock cmpxchgl %2, %1" : "+a"(expected), "+m"(*target) : "r"(desired) : "cc");
        if (expected != 5 || *target != 5) FAIL("cmpxchg: %ld, %ld", expected, *target);
        expected = 1;
        if (fn() != 1) FAIL("call after cmpxchg");
    }
    asm volatile("cld; rep movsb" : "+S"(src), "+D"(dst), "+c"(count) : : "memory", "cc");
    if (fn() != 2) FAIL("call after rep movs");
}

/* mov eax, 34; ret, compiled and run from an RX page, for a case to take away.
 * There is one thread, so nothing depends on timing: a call that does not fault
 * then ran code FEX kept. */
static BYTE *compiled_page(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)page;

    put_code(page, 34);
    protect(page, PAGE, PAGE_EXECUTE_READ);
    set_code(page, PAGE);
    if (fn() != 34) FAIL("first call");
    return page;
}

static void call_gone_page(BYTE *page)
{
    void (*fn)(void) = (void *)page;

    fn();
    check_fault(AV, page, FETCH, page);
}

static void decommit_compiled(void)
{
    BYTE *page = compiled_page();

    if (!VirtualFree(page, PAGE, MEM_DECOMMIT)) FAIL("VirtualFree: %lu", GetLastError());
    call_gone_page(page);
}

/* VirtualAlloc over the committed page, not a VirtualProtect */
static void recommit_noaccess(void)
{
    BYTE *page = compiled_page();

    if (!VirtualAlloc(page, PAGE, MEM_COMMIT, PAGE_NOACCESS)) FAIL("VirtualAlloc: %lu", GetLastError());
    call_gone_page(page);
}

static void decommit_then_noaccess(void)
{
    BYTE *page = compiled_page();

    if (!VirtualFree(page, PAGE, MEM_DECOMMIT)) FAIL("VirtualFree: %lu", GetLastError());
    if (!VirtualAlloc(page, PAGE, MEM_COMMIT, PAGE_NOACCESS)) FAIL("VirtualAlloc: %lu", GetLastError());
    call_gone_page(page);
}

/* An invalid opcode after a mov on an RWX page, which FEX rejects while it
 * decodes the block. */
static void invalid_opcode(void)
{
    static const BYTE code[] = {0xb9, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x04, 0xc3};
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    void (*fn)(void) = (void *)page;

    memcpy(page, code, sizeof(code));
    set_code(page, PAGE);
    fn();
    check_fault(UD, page + 5, 0, NULL);
    if (fault_cx != 1) FAIL("cx %p, wanted 1", (void *)fault_cx);
}

/* A test ah, imm8 that ends the first of two pages, with its immediate on the
 * second. The fault is at the instruction, reported once, with the second page
 * in [1]. */
static BYTE *split_pages(DWORD prot)
{
    static const BYTE test_ah[] = {0xf6, 0xc4};
    BYTE *p = new_pages(2 * PAGE, prot);

    memset(p, 0x90, PAGE);
    memcpy(p + PAGE - sizeof(test_ah), test_ah, sizeof(test_ah));
    set_code(p, 2 * PAGE);
    return p;
}

static void split_run(BYTE *p)
{
    void (*fn)(void) = (void *)(p + PAGE - 2);

    fn();
    check_fault(AV, p + PAGE - 2, FETCH, p + PAGE);
}

static void split_noaccess(void)
{
    BYTE *p = split_pages(PAGE_EXECUTE_READWRITE);

    protect(p + PAGE, PAGE, PAGE_NOACCESS);
    split_run(p);
}

static void split_decommitted(void)
{
    BYTE *p = split_pages(PAGE_EXECUTE_READWRITE);

    if (!VirtualFree(p + PAGE, PAGE, MEM_DECOMMIT)) FAIL("VirtualFree: %lu", GetLastError());
    split_run(p);
}

/* From a page that was never writable and executable at once */
static void split_private(void)
{
    BYTE *p = split_pages(PAGE_READWRITE);

    protect(p, PAGE, PAGE_EXECUTE_READ);
    protect(p + PAGE, PAGE, PAGE_NOACCESS);
    split_run(p);
}

/* 14 prefixes and an escape byte: the opcode is the 16th byte, past the limit of
 * 15, and there the fault is at the start of the instruction. */
static void put_overlong(BYTE *p)
{
    memset(p, 0x66, 14);
    p[14] = 0x0f;
    p[15] = 0x1f;
    p[16] = 0x00;
    p[17] = 0xc3;
}

static void overlong(void)
{
    BYTE *start = new_pages(PAGE, PAGE_EXECUTE_READWRITE) + 0x100;
    void (*fn)(void) = (void *)start;

    put_overlong(start);
    set_code(start, PAGE - 0x100);
    fn();
    check_fault(AV, start, FETCH, start);
}

/* The same on a page where a compile stopped at a guard. The fault is real and
 * the page is executable, which is what a fault on such a page that always ran
 * the instruction again would spin on. Each call must fault once. */
static void guard_then_overlong(DWORD prot)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)page;
    int i, r;

    put_code(page, 33);
    set_code(page, PAGE);
    protect(page, PAGE, prot | PAGE_GUARD);
    r = fn();
    check_guard("first call", page, 33, r);

    protect(page, PAGE, PAGE_READWRITE);
    put_overlong(page);
    protect(page, PAGE, prot);
    for (i = 1; i <= 20; i++)
    {
        fn();
        if (faults != i || fault_params != 2 || fault_info0 != FETCH
                || fault_info1 != (ULONG_PTR)page || fault_pc != (ULONG_PTR)page)
            FAIL("call %d: %ld faults, [0]=%p [1]=%p pc %p, wanted one more at %p", i, faults,
                 (void *)fault_info0, (void *)fault_info1, (void *)fault_pc, page);
    }
    if (guards != 1) FAIL("%ld guard exceptions", guards);
}

static void guard_then_overlong_rx(void)
{
    guard_then_overlong(PAGE_EXECUTE_READ);
}

static void guard_then_overlong_rwx(void)
{
    guard_then_overlong(PAGE_EXECUTE_READWRITE);
}

/* A 15 byte nop, so that a block spans pages. Each page ends with a one byte
 * nop, so no instruction crosses a page, or with the ret. */
static void fill_page(BYTE *p, int last)
{
    static const BYTE nop15[15] = {0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00};
    int i;

    for (i = 0; i + 15 <= PAGE; i += 15) memcpy(p + i, nop15, 15);
    p[PAGE - 1] = last ? 0xc3 : 0x90;
}

static volatile LONG stop, sled_calls;

static DWORD WINAPI sled_caller(void *arg)
{
    void (*fn)(void) = arg;

    while (!stop)
    {
        fn();
        InterlockedIncrement(&sled_calls);
    }
    return 0;
}

/* Takes the last page of a sled away and gives it back while another thread
 * runs and compiles it. The page stays away for a while, so that compiles also
 * find it missing.
 *
 * A fault in the cycles does not show that FEX dropped code: each protect back
 * to RX invalidates the sled, and a decommit can overtake the compile after it.
 * So the page goes once more with the sled compiled. The bytes never change, so
 * only a fetch fault shows the code went: the other thread may finish the calls
 * it is in, and then has to fetch from the page. A call in flight can take a read
 * fault there under FEX_SMCCHECKS=full, where compiled code reads the guest's
 * bytes, and code FEX kept does that every time, so a read does not count. This
 * counts calls, so it does not depend on how fast the machine is. */
static void decommit_race(void)
{
    BYTE *sled = new_pages(SLED_SIZE, PAGE_READWRITE);
    BYTE *page = sled + (SLED_PAGES - 1) * PAGE;
    HANDLE thread;
    LONG calls, ran, before;
    int i;

    for (i = 0; i < SLED_PAGES; i++) fill_page(sled + i * PAGE, i == SLED_PAGES - 1);
    protect(sled, SLED_SIZE, PAGE_EXECUTE_READ);
    set_code(sled, SLED_SIZE);
    thread = CreateThread(NULL, 0, sled_caller, sled, 0, NULL);
    if (!thread) FAIL("CreateThread: %lu", GetLastError());
    while (!sled_calls) YieldProcessor();
    for (i = 0; i < DECOMMIT_CYCLES; i++)
    {
        if (!VirtualFree(page, PAGE, MEM_DECOMMIT)) FAIL("VirtualFree: %lu", GetLastError());
        spin(rand() % 2 ? (unsigned int)rand() * 64u : 0);
        if (!VirtualAlloc(page, PAGE, MEM_COMMIT, PAGE_READWRITE)) FAIL("VirtualAlloc: %lu", GetLastError());
        fill_page(page, 1);
        protect(page, PAGE, PAGE_EXECUTE_READ);
        spin(rand() % 20000);
    }

    /* Two calls, so that one ran whole after the last protect */
    calls = sled_calls;
    while (sled_calls - calls < 2) YieldProcessor();
    before = fetch_faults;
    if (!VirtualFree(page, PAGE, MEM_DECOMMIT)) FAIL("VirtualFree: %lu", GetLastError());
    calls = sled_calls;
    for (;;)
    {
        /* In this order: the handler counts a fetch fault before its call ends,
         * so calls counted here that show no fetch fault below had none. */
        ran = sled_calls;
        if (fetch_faults != before) break;
        if (ran - calls > STALE_CALLS)
            FAIL("%d calls after the decommit and no fetch fault, [1]=%p", STALE_CALLS, (void *)fault_info1);
        YieldProcessor();
    }
    if (fault_info1 != (ULONG_PTR)page) FAIL("[1]=%p, wanted %p", (void *)fault_info1, page);
    stop = 1;
    WaitForSingleObject(thread, INFINITE);
}

static volatile LONG go, done;
static volatile LONG guard_ready, guard_wrong;

static DWORD WINAPI guard_race_thread(void *arg)
{
    int (*fn)(void) = arg;
    LONG seen = 0;

    InterlockedIncrement(&guard_ready);
    while (!stop)
    {
        LONG round = go;

        if (round == seen)
        {
            YieldProcessor();
            continue;
        }
        seen = round;
        if (fn() != 22) InterlockedIncrement(&guard_wrong);
        InterlockedIncrement(&done);
    }
    return 0;
}

/* Threads run into one guard page at once. Windows raises one exception and the
 * code runs in the others, so no thread may fault. With the main thread they all
 * spin, so there are no more of them than a runner of 3 CPUs has room for. */
static void guard_race(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    HANDLE threads[GUARD_THREADS];
    LONG wanted = 0;
    int i;

    put_code(page, 22);
    set_code(page, PAGE);
    for (i = 0; i < GUARD_THREADS; i++)
    {
        threads[i] = CreateThread(NULL, 0, guard_race_thread, page, 0, NULL);
        if (!threads[i]) FAIL("CreateThread: %lu", GetLastError());
    }
    while (guard_ready != GUARD_THREADS) YieldProcessor();
    for (i = 1; i <= GUARD_ROUNDS; i++)
    {
        protect(page, PAGE, PAGE_EXECUTE_READ | PAGE_GUARD);
        wanted += GUARD_THREADS;
        go = i;
        while (done != wanted) YieldProcessor();
    }
    stop = 1;
    WaitForMultipleObjects(GUARD_THREADS, threads, TRUE, INFINITE);
    if (guards != GUARD_ROUNDS) FAIL("%ld guard exceptions, wanted %d", guards, GUARD_ROUNDS);
    if (faults) FAIL("%ld faults in threads that lost the race, [0]=%p [1]=%p", faults,
                     (void *)fault_info0, (void *)fault_info1);
    if (guard_wrong) FAIL("%ld calls returned the wrong value", guard_wrong);
}

static DWORD WINAPI rwx_caller(void *arg)
{
    void (*fn)(void) = arg;
    LONG seen = 0;

    while (!stop)
    {
        if (go == seen)
        {
            YieldProcessor();
            continue;
        }
        seen = go;
        fn();
        done = seen;
    }
    return 0;
}

static double seconds_since(LARGE_INTEGER start)
{
    LARGE_INTEGER now, freq;

    QueryPerformanceCounter(&now);
    QueryPerformanceFrequency(&freq);
    return (double)(now.QuadPart - start.QuadPart) / freq.QuadPart;
}

/* RWX to RW racing a first compile, then a write from the guest, which must not
 * fault: a compile that protects the page after the guest's protect leaves it
 * read-only. That takes the few microseconds after the guest's protect, so the
 * protect is placed by what a compile takes. */
static void rwx_to_rw_race(void)
{
    const char *smc = getenv("FEX_SMCCHECKS");
    int rounds = smc && !strcmp(smc, "full") ? RWX_ROUNDS_FULL : RWX_ROUNDS;
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    void (*fn)(void) = (void *)page;
    double compile_time = 1e9, elapsed;
    LARGE_INTEGER start;
    unsigned int range;
    HANDLE thread;
    int i;

    fill_page(page, 1);
    set_code(page, PAGE);
    for (i = 0; i < 50; i++)
    {
        protect(page, PAGE, PAGE_EXECUTE_READWRITE);
        QueryPerformanceCounter(&start);
        fn();
        elapsed = seconds_since(start);
        if (elapsed < compile_time) compile_time = elapsed;
        protect(page, PAGE, PAGE_READWRITE);
    }
    QueryPerformanceCounter(&start);
    spin(1000000);
    range = (unsigned int)(compile_time * 1.5 / (seconds_since(start) / 1000000)) + 1;

    thread = CreateThread(NULL, 0, rwx_caller, page, 0, NULL);
    if (!thread) FAIL("CreateThread: %lu", GetLastError());
    for (i = 1; i <= rounds; i++)
    {
        protect(page, PAGE, PAGE_EXECUTE_READWRITE);
        go = i;
        spin(((unsigned int)rand() * 32768u + (unsigned int)rand()) % range);
        protect(page, PAGE, PAGE_READWRITE);
        ((volatile BYTE *)page)[0] = 0x66;
        while (done != i) YieldProcessor();
    }
    stop = 1;
    WaitForSingleObject(thread, INFINITE);
}

#define CASE(name) { #name, name }
static const struct
{
    const char *name;
    void (*run)(void);
} cases[] =
{
    CASE(invalid_opcode),
    CASE(split_noaccess),
    CASE(split_decommitted),
    CASE(split_private),
    CASE(overlong),
    CASE(reserved_fall_through),
    CASE(reserved_call),
    CASE(decommit_compiled),
    CASE(recommit_noaccess),
    CASE(decommit_then_noaccess),
    CASE(guard_call),
    CASE(guard_view),
    CASE(guard_fall_through),
    CASE(guard_smc),
    CASE(guard_read_then_call),
    CASE(guard_rearm),
    CASE(guard_noexec),
    CASE(guard_straddle_rx_rx),
    CASE(guard_straddle_rwx_rx),
    CASE(guard_straddle_rwx_rwx),
    CASE(guard_then_overlong_rx),
    CASE(guard_then_overlong_rwx),
    CASE(trapped_writes),
    CASE(guard_race),
    CASE(decommit_race),
    CASE(rwx_to_rw_race),
};

static int selected(const char *name, int argc, char **argv)
{
    int i;

    for (i = 1; i < argc; i++)
        if (!strcmp(argv[i], name)) return 1;
    return argc == 1;
}

int main(int argc, char **argv)
{
    size_t i;
    int ran = 0;

    setvbuf(stdout, NULL, _IONBF, 0);
    srand(GetTickCount());
    AddVectoredExceptionHandler(1, handler);
    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        if (!selected(cases[i].name, argc, argv)) continue;
        printf("wine-build smoke test: %s case %s\n", ARCH, cases[i].name);
        current = cases[i].name;
        code_lo = code_hi = 0;
        faults = guards = stop = go = done = 0;
        cases[i].run();
        ran++;
    }
    if (!ran) FAIL("no such case");
    printf("wine-build smoke test: %s FEX cases passed\n", ARCH);
    return 0;
}
