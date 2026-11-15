/* The program bundle-wine.sh builds for each x86 PE architecture and runs under
 * FEX, once in its default mode and once with FEX_SMCCHECKS=full, and with
 * FEX_DISKCACHE=1 for decommit_race, cross_process_protect and rwx_write_race.
 * Its cases run code in reserved memory and on guard pages, writes to compiled
 * pages, instructions FEX cannot decode or fetch whole, pages taken away or
 * changed while code runs from them, also by another process, which starts this
 * program again as its child, a write to a page while it compiles, and memory
 * syscalls with their out-params on a compiled page, queries of one and writes
 * into one from this process and from a child, code and protections a child
 * changes, and other syscalls that write their buffer on a compiled page. FEX
 * has hung on or got most of these wrong; the others guard behaviour that
 * already worked.
 *
 * The cases run in order in one process, or just those named on the command
 * line; trapped_write_other, cross_process_smc, cross_process_rx,
 * cross_process_restore, cross_process_restore_twice and cross_process_protect
 * also run the program as a child. Each prints its name first. A failed case
 * prints why and ends the process with 1; the usual failure is a hang, and the
 * last line then names the case. There is no watchdog thread, since a stalled
 * JIT stops that as well, so the caller's time limit ends a hang. The exit code
 * is 0 only when every case passed.
 *
 * Do not use TerminateThread on a thread that has just started, which can leave
 * the guest's loader lock held. */

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
#define WRITE_ROUNDS 500
#define GUARD_THREADS 2
#define GUARD_ROUNDS 100
#define CHILD_MS 1000

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
static volatile LONG faults, fetch_faults, read_faults, guards, read_guards;
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
        if (r->ExceptionInformation[0] != FETCH) InterlockedIncrement(&read_guards);
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
    if (r->ExceptionCode == AV && r->ExceptionInformation[0] == 0) InterlockedIncrement(&read_faults);
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

/* The same on an RWX page after an RX one, after a call to code at the end of
 * the RX page, which counts the RWX page among the pages of its code without
 * reading it. A write to the code once it ran has to change it. Only FEX's
 * default mode traps writes for that; FEX_SMCCHECKS=full checks the code of
 * each block instead and passes without the trap. */
static void guard_after_block_smc(void)
{
    BYTE *base = new_pages(2 * PAGE, PAGE_READWRITE);
    int (*first)(void) = (void *)(base + PAGE - 6);
    int (*second)(void) = (void *)(base + PAGE);
    BYTE value;
    int r;

    put_code(base + PAGE - 6, 1);
    put_code(base + PAGE, 2);
    set_code(base, 2 * PAGE);
    protect(base, PAGE, PAGE_EXECUTE_READ);
    protect(base + PAGE, PAGE, PAGE_EXECUTE_READWRITE | PAGE_GUARD);
    r = first();
    if (r != 1 || guards) FAIL("first page: returned %d, %ld guard exceptions", r, guards);
    value = *(volatile BYTE *)(base + PAGE);
    if (guards != 1 || guard_info0 != 0 || value != 0xb8)
        FAIL("read: %ld guard exceptions, [0]=%p value %#x", guards, (void *)guard_info0, value);
    r = second();
    if (r != 2) FAIL("second page: returned %d", r);
    put_code(base + PAGE, 3);
    r = second();
    if (r != 3 || guards != 1 || faults)
        FAIL("after the write: returned %d, %ld guard exceptions, %ld faults", r, guards, faults);
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

/* A function on a page of its own that stores 1 to the byte its argument
 * points at. *insn is the address of the store. */
static void (*store_one(BYTE **insn))(BYTE *)
{
#if defined(__x86_64__)
    /* mov byte [rcx], 1; ret */
    static const BYTE code[] = {0xc6, 0x01, 0x01, 0xc3};
    const int store = 0;
#else
    /* mov eax, [esp+4]; mov byte [eax], 1; ret */
    static const BYTE code[] = {0x8b, 0x44, 0x24, 0x04, 0xc6, 0x00, 0x01, 0xc3};
    const int store = 4;
#endif
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);

    memcpy(page, code, sizeof(code));
    protect(page, PAGE, PAGE_EXECUTE_READ);
    set_code(page, PAGE);
    *insn = page + store;
    return (void *)page;
}

/* A write to a page made RX after RWX is the guest's access violation */
static void rwx_to_rx_write(void)
{
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)page;
    MEMORY_BASIC_INFORMATION info;
    BYTE *insn;
    void (*store)(BYTE *) = store_one(&insn);

    put_code(page, 1);
    protect(page, PAGE, PAGE_EXECUTE_READ);
    store(page + 0x10);
    check_fault(AV, insn, 1, page + 0x10);
    if (page[0x10]) FAIL("the write went through");
    if (!VirtualQuery(page, &info, sizeof(info))) FAIL("VirtualQuery: %lu", GetLastError());
    if (info.Protect != PAGE_EXECUTE_READ) FAIL("protection %#lx after the fault", info.Protect);
    if (fn() != 1) FAIL("call after the fault");
}

/* The same with code compiled from the page before the protect, so with the
 * page trapped */
static void rwx_to_rx_write_compiled(void)
{
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)page;
    BYTE *insn;
    void (*store)(BYTE *) = store_one(&insn);

    put_code(page, 1);
    if (fn() != 1) FAIL("first call");
    protect(page, PAGE, PAGE_EXECUTE_READ);
    store(page + 0x10);
    check_fault(AV, insn, 1, page + 0x10);
    if (page[0x10]) FAIL("the write went through");
    if (fn() != 1) FAIL("call after the fault");
}

/* Three RWX pages made RX together, with code compiled from the middle one:
 * a write to each is the guest's access violation */
static void rwx_to_rx_write_three_pages(void)
{
    BYTE *base = new_pages(3 * PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)(base + PAGE);
    BYTE *insn;
    void (*store)(BYTE *) = store_one(&insn);
    int i;

    put_code(base + PAGE, 1);
    if (fn() != 1) FAIL("first call");
    protect(base, 3 * PAGE, PAGE_EXECUTE_READ);
    for (i = 0; i < 3; i++)
    {
        BYTE *target = base + i * PAGE + 0x10;

        faults = 0;
        store(target);
        check_fault(AV, insn, 1, target);
        if (*target) FAIL("page %d: the write went through", i);
    }
    if (fn() != 1) FAIL("call after the writes");
}

/* A patch of compiled RWX code between a protect and the restore of the
 * protection it reported, which leaves the page writable */
static void rwx_patch_restore(void)
{
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)page;
    DWORD old, tmp;

    put_code(page, 1);
    set_code(page, PAGE);
    if (fn() != 1) FAIL("first call");
    if (!VirtualProtect(page, PAGE, PAGE_EXECUTE_READWRITE, &old))
        FAIL("VirtualProtect: %lu", GetLastError());
    if (old != PAGE_EXECUTE_READWRITE) FAIL("old protection %#lx", old);
    put_code(page, 2);
    if (!VirtualProtect(page, PAGE, old, &tmp)) FAIL("VirtualProtect: %lu", GetLastError());
    if (fn() != 2) FAIL("call after the patch");
    put_code(page, 3);
    if (fn() != 3 || faults) FAIL("call after a write: %ld faults", faults);
}

/* The same with the protection VirtualQuery reported for the trapped page,
 * PAGE_EXECUTE_READWRITE as on Windows */
static void rwx_query_restore(void)
{
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)page;
    MEMORY_BASIC_INFORMATION info;
    DWORD old;

    put_code(page, 1);
    set_code(page, PAGE);
    if (fn() != 1) FAIL("first call");
    if (!VirtualQuery(page, &info, sizeof(info))) FAIL("VirtualQuery: %lu", GetLastError());
    if (info.Protect != PAGE_EXECUTE_READWRITE) FAIL("queried protection %#lx", info.Protect);
    protect(page, PAGE, PAGE_EXECUTE_READWRITE);
    put_code(page, 2);
    if (!VirtualProtect(page, PAGE, info.Protect, &old)) FAIL("VirtualProtect: %lu", GetLastError());
    if (fn() != 2) FAIL("call after the patch");
    put_code(page, 3);
    if (fn() != 3 || faults) FAIL("call after a write: %ld faults", faults);
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

typedef LONG (WINAPI *alloc_fn)(HANDLE, void **, ULONG_PTR, SIZE_T *, ULONG, ULONG);
typedef LONG (WINAPI *free_fn)(HANDLE, void **, SIZE_T *, ULONG);
typedef LONG (WINAPI *protect_fn)(HANDLE, void **, SIZE_T *, ULONG, ULONG *);

static void *ntdll_func(const char *name)
{
    void *p = (void *)GetProcAddress(GetModuleHandleA("ntdll.dll"), name);

    if (!p) FAIL("no %s", name);
    return p;
}

/* An RWX page with mov eax, 1; ret, called once, so that FEX write-protects it.
 * The out-params go at 0x800, after the code. */
static BYTE *trapped_page(void)
{
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)page;

    put_code(page, 1);
    if (fn() != 1) FAIL("first call");
    return page;
}

static void call_again(BYTE *page)
{
    int (*fn)(void) = (void *)page;

    if (fn() != 1) FAIL("call of the trapped page");
}

static DWORD WINAPI alloc_thread(void *arg)
{
    void *p = VirtualAlloc(NULL, PAGE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);

    if (p) VirtualFree(p, 0, MEM_RELEASE);
    return !p;
}

/* A virtual memory call of another thread returns: one that faulted in the
 * syscall can leave Wine's virtual memory mutex held. */
static void check_other_thread(void)
{
    HANDLE thread = CreateThread(NULL, 0, alloc_thread, NULL, 0, NULL);
    DWORD code;

    if (!thread) FAIL("CreateThread: %lu", GetLastError());
    if (WaitForSingleObject(thread, 5000)) FAIL("a VirtualAlloc of another thread did not return");
    if (!GetExitCodeThread(thread, &code) || code) FAIL("VirtualAlloc of another thread failed");
    CloseHandle(thread);
}

/* NtAllocateVirtualMemory and NtFreeVirtualMemory with the address and size on
 * a trapped page. On Windows the page is writable, so both succeed. */
static void trapped_alloc_free(void)
{
    alloc_fn alloc = ntdll_func("NtAllocateVirtualMemory");
    free_fn release = ntdll_func("NtFreeVirtualMemory");
    BYTE *page = trapped_page();
    void **addr = (void **)(page + 0x800);
    SIZE_T *size = (SIZE_T *)(page + 0x800 + sizeof(void *));
    MEMORY_BASIC_INFORMATION info;
    void *mem;
    LONG status;

    *addr = NULL;
    *size = 3 * PAGE;
    call_again(page);
    status = alloc(GetCurrentProcess(), addr, 0, size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
    if (status || !*addr || *size != 3 * PAGE)
        FAIL("alloc: status %#lx, %p size %#lx", status, *addr, (ULONG)*size);
    mem = *addr;
    ((volatile BYTE *)mem)[0] = 1;

    call_again(page);
    *size = 0;
    status = release(GetCurrentProcess(), addr, size, MEM_RELEASE);
    if (status || *addr != mem || *size != 3 * PAGE)
        FAIL("free: status %#lx, %p size %#lx, wanted %p", status, *addr, (ULONG)*size, mem);
    if (!VirtualQuery(mem, &info, sizeof(info)) || info.State != MEM_FREE)
        FAIL("state %#lx after the free", info.State);
    check_other_thread();
    call_again(page);
}

/* NtProtectVirtualMemory with all its out-params on a trapped page, taking the
 * code of an RX page away. The protect succeeds, as on Windows, and the code
 * then faults. VirtualProtect passes old_prot on as it is. */
static void trapped_protect(void)
{
    protect_fn protect_nt = ntdll_func("NtProtectVirtualMemory");
    BYTE *code = compiled_page();
    BYTE *page = trapped_page();
    void **addr = (void **)(page + 0x800);
    SIZE_T *size = (SIZE_T *)(page + 0x800 + sizeof(void *));
    ULONG *old = (ULONG *)(page + 0x800 + 2 * sizeof(void *));
    MEMORY_BASIC_INFORMATION info;
    LONG status;

    *addr = code;
    *size = PAGE;
    call_again(page);
    status = protect_nt(GetCurrentProcess(), addr, size, PAGE_NOACCESS, old);
    if (status || *addr != code || *size != PAGE || *old != PAGE_EXECUTE_READ)
        FAIL("status %#lx, %p size %#lx old %#lx", status, *addr, (ULONG)*size, *old);
    if (!VirtualQuery(code, &info, sizeof(info)) || info.Protect != PAGE_NOACCESS)
        FAIL("protection %#lx after the protect", info.Protect);
    call_gone_page(code);

    call_again(page);
    if (!VirtualProtect(code, PAGE, PAGE_EXECUTE_READ, (DWORD *)old)) FAIL("VirtualProtect: %lu", GetLastError());
    if (*old != PAGE_NOACCESS) FAIL("old protection %#lx", *old);
    check_other_thread();
    call_again(page);
}

static void check_region(BYTE *addr, BYTE *base, SIZE_T size, DWORD prot)
{
    MEMORY_BASIC_INFORMATION info;

    if (!VirtualQuery(addr, &info, sizeof(info))) FAIL("VirtualQuery: %lu", GetLastError());
    if (info.BaseAddress != base || info.RegionSize != size || info.Protect != prot || info.State != MEM_COMMIT)
        FAIL("at %p: base %p size %#lx protection %#lx state %#lx, wanted %p %#lx %#lx", addr, info.BaseAddress,
             (ULONG)info.RegionSize, info.Protect, info.State, base, (ULONG)size, prot);
}

/* VirtualQuery of RWX pages that FEX traps, between one it untrapped and one
 * the guest made RX. Windows reports one RWX region before the RX one. The last
 * query puts its result on a trapped page. */
static void trapped_query(void)
{
    BYTE *base = new_pages(4 * PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)base;
    MEMORY_BASIC_INFORMATION *info = (MEMORY_BASIC_INFORMATION *)(base + 0x800);

    protect(base + 3 * PAGE, PAGE, PAGE_EXECUTE_READ);
    put_code(base, 1);
    if (fn() != 1) FAIL("first call");
    base[2 * PAGE] = 1;

    check_region(base, base, 3 * PAGE, PAGE_EXECUTE_READWRITE);
    check_region(base + PAGE, base + PAGE, 2 * PAGE, PAGE_EXECUTE_READWRITE);
    check_region(base + 2 * PAGE, base + 2 * PAGE, PAGE, PAGE_EXECUTE_READWRITE);
    check_region(base + 3 * PAGE, base + 3 * PAGE, PAGE, PAGE_EXECUTE_READ);

    if (fn() != 1) FAIL("second call");
    if (!VirtualQuery(base, info, sizeof(*info))) FAIL("VirtualQuery into the page: %lu", GetLastError());
    if (info->Protect != PAGE_EXECUTE_READWRITE || info->RegionSize != 3 * PAGE)
        FAIL("into the page: protection %#lx size %#lx", info->Protect, (ULONG)info->RegionSize);
    check_other_thread();
}

/* The same with no page untrapped, between pages the guest made RX before the
 * code ran. Wine can then report the trapped pages and the RX ones as one
 * region. */
static void trapped_query_rx_neighbours(void)
{
    BYTE *base = new_pages(5 * PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)(base + PAGE);

    protect(base, PAGE, PAGE_EXECUTE_READ);
    protect(base + 4 * PAGE, PAGE, PAGE_EXECUTE_READ);
    put_code(base + PAGE, 1);
    if (fn() != 1) FAIL("first call");

    check_region(base, base, PAGE, PAGE_EXECUTE_READ);
    check_region(base + PAGE, base + PAGE, 3 * PAGE, PAGE_EXECUTE_READWRITE);
    check_region(base + 3 * PAGE, base + 3 * PAGE, PAGE, PAGE_EXECUTE_READWRITE);
    check_region(base + 4 * PAGE, base + 4 * PAGE, PAGE, PAGE_EXECUTE_READ);
}

/* WriteProcessMemory into a trapped page of this process. The query reports it
 * writable, as on Windows, so kernelbase writes without a protect, and the
 * wineserver's write has to get through FEX's trap. The code it wrote runs. */
static void trapped_write_self(void)
{
    static const BYTE new_code[6] = {0xb8, 0x02, 0x00, 0x00, 0x00, 0xc3};
    BYTE *page = trapped_page();
    int (*fn)(void) = (void *)page;
    SIZE_T written = 0;

    call_again(page);
    if (!WriteProcessMemory(GetCurrentProcess(), page, new_code, sizeof(new_code), &written))
        FAIL("WriteProcessMemory: %lu", GetLastError());
    if (written != sizeof(new_code)) FAIL("%lu bytes written", (ULONG)written);
    if (fn() != 2) FAIL("call after the write");
    page[0x800] = 1;
    if (fn() != 2 || faults) FAIL("call after a write of the guest: %ld faults", faults);
}

/* The same on a copy-on-write view of a section, where Wine refuses
 * PAGE_EXECUTE_READWRITE and FEX makes the page writable with
 * PAGE_EXECUTE_WRITECOPY */
static void trapped_write_self_cow(void)
{
    static const BYTE new_code[6] = {0xb8, 0x05, 0x00, 0x00, 0x00, 0xc3};
    HANDLE section = CreateFileMappingA(INVALID_HANDLE_VALUE, NULL, PAGE_EXECUTE_READWRITE, 0, PAGE, NULL);
    BYTE *view = section ? MapViewOfFile(section, FILE_MAP_COPY | FILE_MAP_EXECUTE, 0, 0, PAGE) : NULL;
    int (*fn)(void) = (void *)view;
    SIZE_T written = 0;

    if (!view) FAIL("MapViewOfFile: %lu", GetLastError());
    put_code(view, 1);
    if (fn() != 1) FAIL("first call");
    if (fn() != 1) FAIL("second call");
    if (!WriteProcessMemory(GetCurrentProcess(), view, new_code, sizeof(new_code), &written))
        FAIL("WriteProcessMemory: %lu", GetLastError());
    if (written != sizeof(new_code)) FAIL("%lu bytes written", (ULONG)written);
    if (fn() != 5) FAIL("call after the write");
}

/* The child of the cases that change this process from another one, this
 * program run as "<mode> <pid> <address>". It opens this process with the
 * rights Windows asks for the call it makes, which do not let Wine read the
 * handle of FEX's cross-process work list out of this process or duplicate it.
 * wpm_child writes a byte of data, smc_child writes mov eax, 3; ret and flushes
 * the instruction cache, rx_child protects the page PAGE_EXECUTE_READ, and
 * rw_child protects it PAGE_READWRITE, writes mov eax, 3; ret and gives the
 * page back the protection the first protect reported, as a patcher does.
 * rw2_child then does it again with mov eax, 6; ret, and between its second
 * protect and its write waits for this process, see
 * cross_process_restore_twice. denied_child opens this process with
 * PROCESS_QUERY_LIMITED_INFORMATION alone, for which the wineserver must
 * refuse the section of the work list. Returns the exit code. */
static int other_child(int argc, char **argv)
{
    static const BYTE value = 0x5a, code[6] = {0xb8, 0x03, 0x00, 0x00, 0x00, 0xc3};
    static const BYTE code2[6] = {0xb8, 0x06, 0x00, 0x00, 0x00, 0xc3};
    HANDLE process, ready, popped;
    char name[64];
    SIZE_T written = 0;
    DWORD pid, old, tmp;
    void *addr;

    if (argc != 4) return 2;
    pid = strtoul(argv[2], NULL, 10);
    addr = (void *)(ULONG_PTR)strtoull(argv[3], NULL, 16);
    if (!strcmp(argv[1], "denied_child"))
    {
#ifdef __x86_64__
        /* ProcessWineCrossProcessWorkSection, which wow64 has no thunk for */
        LONG (WINAPI *query)(HANDLE, ULONG, void *, ULONG, ULONG *) = ntdll_func("NtQueryInformationProcess");
        HANDLE section = 0;
        LONG status;

        if (!(process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid))) return 3;
        status = query(process, 1003, &section, sizeof(section), NULL);
        if (status != (LONG)0xc0000022 || section) return 4;
#endif
        return 0;
    }
    if (!strcmp(argv[1], "rx_child"))
    {
        if (!(process = OpenProcess(PROCESS_VM_OPERATION, FALSE, pid))) return 3;
        return VirtualProtectEx(process, addr, PAGE, PAGE_EXECUTE_READ, &old) ? 0 : 4;
    }
    /* kernelbase's WriteProcessMemory also queries the page */
    process = OpenProcess(PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION, FALSE, pid);
    if (!process) return 3;
    if (!strcmp(argv[1], "rw_child") || !strcmp(argv[1], "rw2_child"))
    {
        if (!VirtualProtectEx(process, addr, PAGE, PAGE_READWRITE, &old)) return 4;
        if (!WriteProcessMemory(process, addr, code, sizeof(code), &written) || written != sizeof(code)) return 5;
        if (!VirtualProtectEx(process, addr, PAGE, old, &tmp)) return 6;
        if (!strcmp(argv[1], "rw_child")) return 0;
        snprintf(name, sizeof(name), "fex-smoke-ready-%lu", pid);
        if (!(ready = OpenEventA(EVENT_MODIFY_STATE, FALSE, name))) return 7;
        snprintf(name, sizeof(name), "fex-smoke-popped-%lu", pid);
        if (!(popped = OpenEventA(SYNCHRONIZE, FALSE, name))) return 7;
        if (!VirtualProtectEx(process, addr, PAGE, PAGE_READWRITE, &old)) return 4;
        SetEvent(ready);
        if (WaitForSingleObject(popped, 20000)) return 8;
        if (!WriteProcessMemory(process, addr, code2, sizeof(code2), &written) || written != sizeof(code2)) return 5;
        return VirtualProtectEx(process, addr, PAGE, old, &tmp) ? 0 : 6;
    }
    if (!strcmp(argv[1], "smc_child"))
    {
        if (!WriteProcessMemory(process, addr, code, sizeof(code), &written) || written != sizeof(code)) return 4;
        return FlushInstructionCache(process, addr, sizeof(code)) ? 0 : 5;
    }
    if (!WriteProcessMemory(process, addr, &value, 1, &written)) return 4;
    return written == 1 ? 0 : 5;
}

/* Starts other_child on the address */
static HANDLE start_child(const char *mode, void *addr)
{
    char path[MAX_PATH], cmdline[MAX_PATH + 64];
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi;

    if (!GetModuleFileNameA(NULL, path, sizeof(path))) FAIL("GetModuleFileName: %lu", GetLastError());
    snprintf(cmdline, sizeof(cmdline), "\"%s\" %s %lu %p", path, mode, GetCurrentProcessId(), addr);
    if (!CreateProcessA(path, cmdline, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi))
        FAIL("CreateProcess: %lu", GetLastError());
    CloseHandle(pi.hThread);
    return pi.hProcess;
}

static void end_child(const char *mode, HANDLE process)
{
    DWORD code;

    if (WaitForSingleObject(process, 20000)) FAIL("the %s did not end", mode);
    if (!GetExitCodeProcess(process, &code) || code) FAIL("the %s exited with %lu", mode, code);
    CloseHandle(process);
}

/* Runs other_child on the address and waits for it. This process compiles
 * nothing meanwhile. */
static void run_child(const char *mode, void *addr)
{
    end_child(mode, start_child(mode, addr));
}

/* Code compiled now has FEX process the work other processes queued first */
static void compile_new_code(void)
{
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    int (*fn)(void) = (void *)page;

    put_code(page, 4);
    if (fn() != 4) FAIL("call of new code");
}

/* The same from another process, with wpm_child. Its query of this process
 * reports FEX's trap, so kernelbase makes the page RWX, writes and restores RX,
 * which FEX takes for queued protects. On Windows the page stays RWX, so this
 * process's own write after it must not fault. */
static void trapped_write_other(void)
{
    BYTE *page = trapped_page();
    int (*fn)(void) = (void *)page;

    call_again(page);
    run_child("wpm_child", page + 0x10);
    compile_new_code();
    if (page[0x10] != 0x5a) FAIL("byte %#x after the child's write", page[0x10]);
    page[0x800] = 1;
    if (fn() != 1 || faults) FAIL("call after a write of the guest: %ld faults", faults);
}

/* Code on a trapped page that smc_child replaces. This process runs the new
 * code, as on Windows once WriteProcessMemory and FlushInstructionCache have
 * returned. denied_child then checks the rights the work list needs. */
static void cross_process_smc(void)
{
    BYTE *page = trapped_page();
    int (*fn)(void) = (void *)page;
    int r;

    call_again(page);
    run_child("smc_child", page);
    compile_new_code();
    if (page[1] != 3) FAIL("byte %#x after the child's write", page[1]);
    if ((r = fn()) != 3) FAIL("the call after the child's write returned %d, wanted 3", r);
    run_child("denied_child", page);
}

/* A trapped page that rw_child patches through PAGE_READWRITE and gives back
 * the trap's PAGE_EXECUTE_READ, which its protect reported. On Windows that
 * protect reported PAGE_EXECUTE_READWRITE, so this process's own write
 * afterwards must not fault, and the new code runs. */
static void cross_process_restore(void)
{
    BYTE *page = trapped_page();
    int (*fn)(void) = (void *)page;
    int r;

    call_again(page);
    run_child("rw_child", page);
    compile_new_code();
    page[0x800] = 1;
    if ((r = fn()) != 3 || faults) FAIL("the call after the patch returned %d, wanted 3, %ld faults", r, faults);
}

/* The same patch twice, with rw2_child, which waits after its second protect
 * to PAGE_READWRITE until this process has compiled new code, which processes
 * the work queued so far: the first restore then runs while the page is
 * writable on the host, and the second restore must still give the page
 * back. */
static void cross_process_restore_twice(void)
{
    BYTE *page = trapped_page();
    int (*fn)(void) = (void *)page;
    HANDLE ready, popped, child;
    char name[64];
    int r;

    snprintf(name, sizeof(name), "fex-smoke-ready-%lu", GetCurrentProcessId());
    if (!(ready = CreateEventA(NULL, TRUE, FALSE, name))) FAIL("CreateEvent: %lu", GetLastError());
    snprintf(name, sizeof(name), "fex-smoke-popped-%lu", GetCurrentProcessId());
    if (!(popped = CreateEventA(NULL, TRUE, FALSE, name))) FAIL("CreateEvent: %lu", GetLastError());
    call_again(page);
    child = start_child("rw2_child", page);
    if (WaitForSingleObject(ready, 20000)) FAIL("the rw2_child did not get to its second patch");
    compile_new_code();
    SetEvent(popped);
    end_child("rw2_child", child);
    compile_new_code();
    page[0x800] = 1;
    if ((r = fn()) != 6 || faults) FAIL("the call after the patches returned %d, wanted 6, %ld faults", r, faults);
    CloseHandle(ready);
    CloseHandle(popped);
}

/* An RWX page that rx_child makes PAGE_EXECUTE_READ, first with no code
 * compiled from it, then trapped. A write of this process to it is then the
 * access violation, as on Windows. */
static void cross_process_rx(void)
{
    BYTE *insn;
    void (*store)(BYTE *) = store_one(&insn);
    int trapped;

    for (trapped = 0; trapped < 2; trapped++)
    {
        const char *what = trapped ? "trapped" : "not trapped";
        BYTE *page = trapped ? trapped_page() : new_pages(PAGE, PAGE_EXECUTE_READWRITE);
        int (*fn)(void) = (void *)page;

        if (!trapped) put_code(page, 1);
        run_child("rx_child", page);
        compile_new_code();
        faults = 0;
        store(page + 0x10);
        if (faults != 1 || fault_code != AV || fault_pc != (ULONG_PTR)insn || fault_info0 != 1
                || fault_info1 != (ULONG_PTR)(page + 0x10))
            FAIL("%s: %ld faults, code %#lx pc %p [0]=%p [1]=%p, wanted one write fault at %p", what, faults,
                 fault_code, (void *)fault_pc, (void *)fault_info0, (void *)fault_info1, page + 0x10);
        if (page[0x10]) FAIL("%s: the write went through", what);
        if (fn() != 1) FAIL("%s: call after the fault", what);
    }
}

typedef LONG (WINAPI *read_fn)(HANDLE, const void *, void *, SIZE_T, SIZE_T *);
typedef LONG (WINAPI *query_system_fn)(ULONG, void *, ULONG, ULONG *);

/* NtReadVirtualMemory and NtQuerySystemInformation(SystemTimeOfDayInformation)
 * with their buffer on a trapped page, which is writable on Windows, so both
 * succeed. Each call has the page trapped again before it. */
static void trapped_syscall_buffer(void)
{
    static const char data[16] = "read into code";
    read_fn read_nt = ntdll_func("NtReadVirtualMemory");
    query_system_fn query_nt = ntdll_func("NtQuerySystemInformation");
    BYTE *page = trapped_page();
    LARGE_INTEGER *times = (LARGE_INTEGER *)(page + 0x800);
    ULARGE_INTEGER now;
    FILETIME ft;
    SIZE_T done = 0;
    LONG status;

    call_again(page);
    status = read_nt(GetCurrentProcess(), data, page + 0x800, sizeof(data), &done);
    if (status || done != sizeof(data) || memcmp(page + 0x800, data, sizeof(data)))
        FAIL("NtReadVirtualMemory: status %#lx, %lu bytes", status, (ULONG)done);

    GetSystemTimeAsFileTime(&ft);
    now.LowPart = ft.dwLowDateTime;
    now.HighPart = ft.dwHighDateTime;
    call_again(page);
    /* SystemTimeOfDayInformation: boot time, system time, time zone bias, id */
    status = query_nt(3, times, 4 * sizeof(*times), NULL);
    if (status) FAIL("NtQuerySystemInformation: status %#lx", status);
    if ((ULONGLONG)times[1].QuadPart < now.QuadPart)
        FAIL("system time %#llx, before %#llx", (ULONGLONG)times[1].QuadPart, now.QuadPart);
    call_again(page);
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
 * it is in, and then has to fetch from the page. Under FEX_SMCCHECKS=full,
 * compiled code reads the guest's bytes before each instruction, and a fault in
 * that read must reach the guest as the instruction's fetch fault, so a read
 * fault fails the case. A call in flight on the page then faults at its next
 * instruction, anywhere in the page, as on Windows, so any fetch fault on the
 * page passes; the first one's offset is printed. This counts calls, so it does
 * not depend on how fast the machine is. */
static void decommit_race(void)
{
    BYTE *sled = new_pages(SLED_SIZE, PAGE_READWRITE);
    BYTE *page = sled + (SLED_PAGES - 1) * PAGE;
    HANDLE thread;
    LONG calls, ran, before;
    ULONG_PTR first;
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
    first = fault_info1;
    stop = 1;
    WaitForSingleObject(thread, INFINITE);
    /* The first fault and, with the thread stopped, the last one */
    if (first - (ULONG_PTR)page >= PAGE || fault_info1 - (ULONG_PTR)page >= PAGE)
        FAIL("[1]=%p, last %p, wanted addresses on %p", (void *)first, (void *)fault_info1, page);
    if (read_faults) FAIL("%ld read faults in the sled", read_faults);
    printf("wine-build smoke test: %s decommit_race fetch fault at page offset %#lx\n", ARCH,
           (unsigned long)(first - (ULONG_PTR)page));
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

static DWORD WINAPI guard_caller(void *arg)
{
    int (*fn)(void) = arg;

    while (!stop)
        if (fn() != 23) InterlockedIncrement(&guard_wrong);
    return 0;
}

/* Arms the guard of a page while another thread keeps calling its code, so a
 * call can be in code compiled before the arm. Each arm raises one guard
 * exception, at the fetch of an instruction on the page, and nothing faults.
 * Under FEX_SMCCHECKS=full that code reads the page before it runs it, and the
 * read stands in for the fetch. */
static void guard_while_running(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    HANDLE thread;
    int i;

    put_code(page, 23);
    protect(page, PAGE, PAGE_EXECUTE_READ);
    set_code(page, PAGE);
    thread = CreateThread(NULL, 0, guard_caller, page, 0, NULL);
    if (!thread) FAIL("CreateThread: %lu", GetLastError());
    for (i = 1; i <= GUARD_ROUNDS; i++)
    {
        protect(page, PAGE, PAGE_EXECUTE_READ | PAGE_GUARD);
        while (guards != i && !faults) YieldProcessor();
        if (faults) break;
    }
    stop = 1;
    WaitForSingleObject(thread, INFINITE);
    if (faults) FAIL("%ld faults, [0]=%p [1]=%p", faults, (void *)fault_info0, (void *)fault_info1);
    /* At either instruction: under FEX_SMCCHECKS=full an arm can land between
     * the reads of the mov and the ret */
    if (read_guards || guard_info1 != guard_pc
            || (guard_pc != (ULONG_PTR)page && guard_pc != (ULONG_PTR)page + 5))
        FAIL("%ld of %ld guard exceptions not a fetch, last [0]=%p [1]=%p pc %p", read_guards, guards,
             (void *)guard_info0, (void *)guard_info1, (void *)guard_pc);
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

/* How many rounds of spin() one and a half first compiles of the page take, for
 * a race to place something in a compile another thread runs. Leaves the page
 * RW. */
static unsigned int compile_spins(BYTE *page)
{
    void (*fn)(void) = (void *)page;
    double compile_time = 1e9, elapsed;
    LARGE_INTEGER start;
    int i;

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
    return (unsigned int)(compile_time * 1.5 / (seconds_since(start) / 1000000)) + 1;
}

static void spin_below(unsigned int range)
{
    spin(((unsigned int)rand() * 32768u + (unsigned int)rand()) % range);
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
    unsigned int range;
    HANDLE thread;
    int i;

    fill_page(page, 1);
    set_code(page, PAGE);
    range = compile_spins(page);

    thread = CreateThread(NULL, 0, rwx_caller, page, 0, NULL);
    if (!thread) FAIL("CreateThread: %lu", GetLastError());
    for (i = 1; i <= rounds; i++)
    {
        protect(page, PAGE, PAGE_EXECUTE_READWRITE);
        go = i;
        spin_below(range);
        protect(page, PAGE, PAGE_READWRITE);
        ((volatile BYTE *)page)[0] = 0x66;
        while (done != i) YieldProcessor();
    }
    stop = 1;
    WaitForSingleObject(thread, INFINITE);
}

/* A write to an RWX page racing a first compile of it, then a call from the
 * writing thread, which must run what it wrote. FEX write-protects the page at
 * the end of the compile, so a write after the compile read the code takes no
 * fault that would drop the block. The write is placed by what a compile takes,
 * and the code fills the page so that a compile takes long enough to hit. Under
 * FEX_SMCCHECKS=full only a write while the compile decodes is lost, about one
 * round in 30, which sets the number of rounds. */
static void rwx_write_race(void)
{
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    int (*fn)(void) = (void *)page;
    unsigned int range;
    HANDLE thread;
    int i, r;

    /* mov eax, 0; ten one byte nops; the sled; ret */
    fill_page(page, 1);
    put_code(page, 0);
    memset(page + 5, 0x90, 10);
    set_code(page, PAGE);
    range = compile_spins(page);

    thread = CreateThread(NULL, 0, rwx_caller, page, 0, NULL);
    if (!thread) FAIL("CreateThread: %lu", GetLastError());
    for (i = 1; i <= WRITE_ROUNDS; i++)
    {
        protect(page, PAGE, PAGE_EXECUTE_READWRITE);
        go = i;
        spin_below(range);
        memcpy(page + 1, &i, 4);
        while (done != i) YieldProcessor();
        r = fn();
        if (r != i) FAIL("round %d: the call after the write returned %d", i, r);
    }
    stop = 1;
    WaitForSingleObject(thread, INFINITE);
}

/* The child of cross_process_protect, started as "child <pid> <page> <range>":
 * for CHILD_MS, takes the page of that process away, turn by turn as NOACCESS
 * and as a guard page, and gives it back, each after a spin below <range>. The
 * parent compiles the page for every call, and another process does not wait
 * for that compile, so the page also goes while it reads the page. Returns the
 * exit code. */
static int child_protect(char **argv)
{
    DWORD pid = strtoul(argv[2], NULL, 0);
    void *page = (void *)(ULONG_PTR)strtoull(argv[3], NULL, 0);
    unsigned int range = strtoul(argv[4], NULL, 0);
    HANDLE process = OpenProcess(PROCESS_VM_OPERATION, FALSE, pid);
    DWORD start = GetTickCount(), old;
    int i;

    if (!process) return 2;
    for (i = 0; GetTickCount() - start < CHILD_MS; i++)
    {
        if (!VirtualProtectEx(process, page, PAGE, i % 2 ? PAGE_EXECUTE_READ | PAGE_GUARD : PAGE_NOACCESS, &old))
            return 3;
        spin_below(range);
        if (!VirtualProtectEx(process, page, PAGE, PAGE_EXECUTE_READ, &old)) return 4;
        spin_below(range);
    }
    return 0;
}

static DWORD WINAPI idle_waiter(void *arg)
{
    WaitForSingleObject(arg, INFINITE);
    return 0;
}

/* Calls the code of a page while a child process takes the page away and gives
 * it back. Wine runs the child's protect in a thread of this process that
 * waits, which a thread that only waits provides, so the page can go while the
 * calling thread compiles it. Each fault and guard exception is the fetch of an
 * instruction on the page, and the call after the child ends runs. */
static void cross_process_protect(void)
{
    HANDLE idle = CreateEventA(NULL, TRUE, FALSE, NULL), waiter;
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    void (*fn)(void) = (void *)page;
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi;
    char path[MAX_PATH], cmd[MAX_PATH + 96];
    LONG before, before_guards;
    unsigned int range;
    DWORD code;

    fill_page(page, 1);
    set_code(page, PAGE);
    range = compile_spins(page);
    protect(page, PAGE, PAGE_EXECUTE_READ);
    if (!idle || !(waiter = CreateThread(NULL, 0, idle_waiter, idle, 0, NULL)))
        FAIL("CreateThread: %lu", GetLastError());
    if (!GetModuleFileNameA(NULL, path, sizeof(path))) FAIL("GetModuleFileName: %lu", GetLastError());
    snprintf(cmd, sizeof(cmd), "\"%s\" child %lu %llu %u", path, GetCurrentProcessId(),
             (unsigned long long)(ULONG_PTR)page, range);
    if (!CreateProcessA(path, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi))
        FAIL("CreateProcess: %lu", GetLastError());
    while (WaitForSingleObject(pi.hProcess, 0) == WAIT_TIMEOUT)
    {
        before = faults;
        before_guards = guards;
        /* Drops the compiled code, so every call compiles the page */
        FlushInstructionCache(GetCurrentProcess(), page, PAGE);
        fn();
        if (faults != before && (fault_code != AV || fault_info0 != FETCH || fault_info1 != fault_pc))
            FAIL("fault %#lx, [0]=%p [1]=%p pc %p, wanted a fetch fault", fault_code, (void *)fault_info0,
                 (void *)fault_info1, (void *)fault_pc);
        if (guards != before_guards && (guard_info0 != FETCH || guard_info1 != guard_pc
                || guard_pc < (ULONG_PTR)page || guard_pc >= (ULONG_PTR)page + PAGE))
            FAIL("guard exception [0]=%p [1]=%p pc %p, wanted the fetch of an instruction on %p",
                 (void *)guard_info0, (void *)guard_info1, (void *)guard_pc, page);
    }
    if (!GetExitCodeProcess(pi.hProcess, &code) || code) FAIL("the child exited with %lu", code);
    before = faults;
    fn();
    if (faults != before) FAIL("the call after the child faulted, [0]=%p [1]=%p", (void *)fault_info0,
                               (void *)fault_info1);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    SetEvent(idle);
    WaitForSingleObject(waiter, INFINITE);
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
    CASE(guard_after_block_smc),
    CASE(guard_rearm),
    CASE(guard_noexec),
    CASE(guard_straddle_rx_rx),
    CASE(guard_straddle_rwx_rx),
    CASE(guard_straddle_rwx_rwx),
    CASE(guard_then_overlong_rx),
    CASE(guard_then_overlong_rwx),
    CASE(trapped_writes),
    CASE(rwx_to_rx_write),
    CASE(rwx_to_rx_write_compiled),
    CASE(rwx_to_rx_write_three_pages),
    CASE(rwx_patch_restore),
    CASE(rwx_query_restore),
    CASE(trapped_alloc_free),
    CASE(trapped_protect),
    CASE(trapped_query),
    CASE(trapped_query_rx_neighbours),
    CASE(trapped_write_self),
    CASE(trapped_write_self_cow),
    CASE(trapped_write_other),
    CASE(cross_process_smc),
    CASE(cross_process_rx),
    CASE(cross_process_restore),
    CASE(cross_process_restore_twice),
    CASE(trapped_syscall_buffer),
    CASE(guard_race),
    CASE(guard_while_running),
    CASE(decommit_race),
    CASE(cross_process_protect),
    CASE(rwx_to_rw_race),
    CASE(rwx_write_race),
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

    if (argc > 1 && (!strcmp(argv[1], "wpm_child") || !strcmp(argv[1], "smc_child") || !strcmp(argv[1], "rx_child")
                     || !strcmp(argv[1], "rw_child") || !strcmp(argv[1], "rw2_child")
                     || !strcmp(argv[1], "denied_child")))
        return other_child(argc, argv);
    setvbuf(stdout, NULL, _IONBF, 0);
    srand(GetTickCount());
    if (argc == 5 && !strcmp(argv[1], "child")) return child_protect(argv);
    AddVectoredExceptionHandler(1, handler);
    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        if (!selected(cases[i].name, argc, argv)) continue;
        printf("wine-build smoke test: %s case %s\n", ARCH, cases[i].name);
        current = cases[i].name;
        code_lo = code_hi = 0;
        faults = read_faults = guards = read_guards = guard_wrong = stop = go = done = 0;
        cases[i].run();
        ran++;
    }
    if (!ran) FAIL("no such case");
    printf("wine-build smoke test: %s FEX cases passed\n", ARCH);
    return 0;
}
