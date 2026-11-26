/* The program bundle-wine.sh builds for each x86 PE architecture and runs under
 * FEX, in its default mode, with FEX_SMCCHECKS=full and with full precision x87,
 * and with FEX_DISKCACHE=1, alone and with FEX_SMCCHECKS=full, for
 * decommit_race, cross_process_protect, rwx_write_race,
 * cross_process_write_race and rx_to_rwx_race. Its cases run code in reserved
 * memory and on guard pages, writes to compiled pages, instructions FEX cannot
 * decode or fetch whole, pages taken away or changed while code runs from them,
 * also by another process, which starts this program again as its child, a
 * write to a page while it compiles, also from another process, a page made
 * writable while it compiles, memory syscalls with their out-params on a
 * compiled page, queries of one and writes into one from this process and from
 * a child, code and protections a child changes, other syscalls that write
 * their buffer on a compiled page, x87 code that a fault or a write to its own
 * page interrupts, and x87 operations that must leave the callee-saved
 * registers alone. FEX has hung on or got most of these wrong; the
 * others guard behaviour that already worked.
 *
 * The cases run in order in one process, or just those named on the command
 * line; trapped_write_other, cross_process_smc, cross_process_rx,
 * cross_process_restore, cross_process_restore_twice, cross_process_protect and
 * cross_process_write_race also run the program as a child. Each prints its
 * name first. A failed case prints why and ends the process with 1; the usual
 * failure is a hang, and the last line then names the case. There is no
 * watchdog thread, since a stalled JIT stops that as well, so the caller's time
 * limit ends a hang. The exit code is 0 only when every case passed.
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
#define WRITE_RACE_ROUNDS 20
#define GUARD_THREADS 2
#define GUARD_ROUNDS 100
#define CHILD_MS 1000
/* cross_process_protect: how many protects it keeps the times of, and of how
 * many of its last calls, which start long before the child's first protect */
#define PROTECT_TIMES 16384
#define CALL_TIMES 65536

/* cross_process_write_race: the sleds it compiles first, for how long a
 * compile takes, the size of each, the offset of its mov and its addps count,
 * lower under FEX_SMCCHECKS=full, which takes about 80 times as long to
 * compile each. A compile of either takes 0.5 ms or more on a current Mac, and
 * the child's write about 0.15 ms. */
#define WRITE_RACE_SLEDS 3
#define WRITE_RACE_SIZE (4 * PAGE)
#define WRITE_RACE_MOV (8 * 15)
#define WRITE_RACE_ADDS 4800
#define WRITE_RACE_ADDS_FULL 1200

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

static void CALLBACK end_sleep(ULONG_PTR arg)
{
}

/* Runs the queries and protects another process makes in this one. The
 * wineserver gives them, as system APCs, to a thread of this process that
 * waits in the server, and only without one signals a thread, which can be the
 * one that compiles. Wine's waits on events stay in this process, an alertable
 * SleepEx waits in the server. A user APC ends the thread once *arg is set. */
static DWORD WINAPI apc_sleeper(void *arg)
{
    while (!*(volatile LONG *)arg) SleepEx(INFINITE, TRUE);
    return 0;
}

/* The child of cross_process_protect, started as "child <pid> <page> <range>
 * <times>": for CHILD_MS, takes the page of that process away, turn by turn as
 * NOACCESS and as a guard page, and gives it back, each after a spin below
 * <range>. At the end it writes the number of protects that took the page away
 * to <times> in that process, followed by the QueryPerformanceCounter values
 * from before and after each of them, of the first PROTECT_TIMES. Returns the
 * exit code. */
static int child_protect(char **argv)
{
    static LONGLONG times[1 + 2 * PROTECT_TIMES];
    DWORD pid = strtoul(argv[2], NULL, 0);
    void *page = (void *)(ULONG_PTR)strtoull(argv[3], NULL, 0);
    unsigned int range = strtoul(argv[4], NULL, 0);
    void *dest = (void *)(ULONG_PTR)strtoull(argv[5], NULL, 0);
    HANDLE process = OpenProcess(PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_QUERY_INFORMATION, FALSE, pid);
    DWORD start = GetTickCount(), old;
    LARGE_INTEGER before, after;
    SIZE_T size;
    int i, n = 0;

    if (!process) return 2;
    for (i = 0; GetTickCount() - start < CHILD_MS; i++)
    {
        QueryPerformanceCounter(&before);
        if (!VirtualProtectEx(process, page, PAGE, i % 2 ? PAGE_EXECUTE_READ | PAGE_GUARD : PAGE_NOACCESS, &old))
            return 3;
        QueryPerformanceCounter(&after);
        if (n < PROTECT_TIMES)
        {
            times[1 + 2 * n] = before.QuadPart;
            times[2 + 2 * n] = after.QuadPart;
            n++;
        }
        spin_below(range);
        if (!VirtualProtectEx(process, page, PAGE, PAGE_EXECUTE_READ, &old)) return 4;
        spin_below(range);
    }
    times[0] = n;
    if (!WriteProcessMemory(process, dest, times, (1 + 2 * n) * sizeof(times[0]), &size)
            || size != (1 + 2 * n) * sizeof(times[0]))
        return 5;
    return 0;
}

/* Calls the code of a page while a child process takes the page away and gives
 * it back, and checks that each fault and guard exception is the fetch of an
 * instruction on the page and that the call after the child ends runs. Every
 * call compiles the page. Wine runs the child's protects in apc_sleeper, so the
 * page can go while this thread compiles it. FEX learns of each protect later,
 * when a thread processes the work the child queued, at a syscall or at the
 * start of a compile. It prints how many of the protects that took the page
 * away during the calls it kept the times of began and ended inside a call,
 * whose time is mostly its compile. Under FEX_SMCCHECKS=full without
 * FEX_DISKCACHE=1, where every call compiles and a compile takes longer than a
 * protect of the child, the case fails when none did, as it then checked
 * nothing. With the disk cache most calls load a block instead, and in the
 * other modes a call is shorter than such a protect, so the count stays small
 * there and is only printed. */
static void cross_process_protect(void)
{
    static LONGLONG times[1 + 2 * PROTECT_TIMES], calls[2 * CALL_TIMES];
    const char *smc = getenv("FEX_SMCCHECKS"), *disk_cache = getenv("FEX_DISKCACHE");
    BYTE *page = new_pages(PAGE, PAGE_READWRITE);
    void (*fn)(void) = (void *)page;
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi;
    char path[MAX_PATH], cmd[MAX_PATH + 128];
    volatile LONG done_sleeping = 0;
    LONG before, before_guards;
    LARGE_INTEGER call_start, call_end;
    unsigned int range;
    HANDLE waiter;
    DWORD code;
    int n, j, k, first, kept, ncalls = 0, seen = 0, inside = 0;

    fill_page(page, 1);
    set_code(page, PAGE);
    range = compile_spins(page);
    protect(page, PAGE, PAGE_EXECUTE_READ);
    if (!(waiter = CreateThread(NULL, 0, apc_sleeper, (void *)&done_sleeping, 0, NULL)))
        FAIL("CreateThread: %lu", GetLastError());
    if (!GetModuleFileNameA(NULL, path, sizeof(path))) FAIL("GetModuleFileName: %lu", GetLastError());
    snprintf(cmd, sizeof(cmd), "\"%s\" child %lu %llu %u %llu", path, GetCurrentProcessId(),
             (unsigned long long)(ULONG_PTR)page, range, (unsigned long long)(ULONG_PTR)times);
    if (!CreateProcessA(path, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi))
        FAIL("CreateProcess: %lu", GetLastError());
    while (WaitForSingleObject(pi.hProcess, 0) == WAIT_TIMEOUT)
    {
        before = faults;
        before_guards = guards;
        /* Drops the compiled code, so every call compiles the page */
        FlushInstructionCache(GetCurrentProcess(), page, PAGE);
        QueryPerformanceCounter(&call_start);
        fn();
        QueryPerformanceCounter(&call_end);
        calls[2 * (ncalls % CALL_TIMES)] = call_start.QuadPart;
        calls[2 * (ncalls % CALL_TIMES) + 1] = call_end.QuadPart;
        ncalls++;
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
    /* The calls of the ring from the oldest one on. Both lists are in time
     * order and the calls do not overlap: the call a protect can lie in is the
     * last one that began before it. Only the protects after the oldest call
     * kept count. */
    n = (int)times[0];
    first = ncalls > CALL_TIMES ? ncalls % CALL_TIMES : 0;
    kept = ncalls > CALL_TIMES ? CALL_TIMES : ncalls;
#define CALL_AT(j) (calls + 2 * ((first + (j)) % CALL_TIMES))
    for (k = 0, j = 0; k < n && kept; k++)
    {
        if (times[1 + 2 * k] <= CALL_AT(0)[0]) continue;
        seen++;
        while (j + 1 < kept && CALL_AT(j + 1)[0] < times[1 + 2 * k]) j++;
        inside += times[2 + 2 * k] < CALL_AT(j)[1];
    }
#undef CALL_AT
    printf("wine-build smoke test: %s cross_process_protect %d of %d protects that took the page away during the"
           " last %d calls ran inside a call\n", ARCH, inside, seen, kept);
    if (!inside && smc && !strcmp(smc, "full") && !(disk_cache && !strcmp(disk_cache, "1")))
        FAIL("no protect of the child ran inside a call");
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    done_sleeping = 1;
    if (!QueueUserAPC(end_sleep, waiter, 0)) FAIL("QueueUserAPC: %lu", GetLastError());
    WaitForSingleObject(waiter, INFINITE);
    CloseHandle(waiter);
}

/* The sled of cross_process_write_race with index i */
static BYTE *race_sled(BYTE *sleds, int i)
{
    return sleds + (SIZE_T)i * WRITE_RACE_SIZE;
}

/* Eight 15 byte nops, mov eax, value, ten one byte nops, adds addps xmm1, xmm2,
 * which take a compile longer than nops, and ret. The mov is past the bytes by
 * which FEX's disk cache looks up anonymous code, about 64, so all sleds have
 * one disk cache key, and the block stays below FEX's 5000 instructions. */
static void race_fill(BYTE *p, int value, int adds)
{
    static const BYTE nop15[15] = {0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x0f, 0x1f, 0x84,
                                   0x00, 0x00, 0x00, 0x00, 0x00};
    static const BYTE addps[3] = {0x0f, 0x58, 0xca};
    int i;

    memset(p, 0xcc, WRITE_RACE_SIZE);
    for (i = 0; i < WRITE_RACE_MOV; i += 15) memcpy(p + i, nop15, 15);
    p[WRITE_RACE_MOV] = 0xb8;
    memcpy(p + WRITE_RACE_MOV + 1, &value, sizeof(value));
    memset(p + WRITE_RACE_MOV + 5, 0x90, 10);
    p += WRITE_RACE_MOV + 15;
    for (i = 0; i < adds; i++, p += 3) memcpy(p, addps, 3);
    *p = 0xc3;
}

/* The child of cross_process_write_race, started as "write_child <pid> <sleds>
 * <range> <times>": in each round, once that process signals, writes 2 * i + 1
 * over the mov's immediate in sled i of the round, after a spin below <range>,
 * stores the QueryPerformanceCounter values from before and after the write
 * at <times> in that process, 2 per round, and signals back. It makes the page
 * PAGE_EXECUTE_READWRITE for the write and gives it back PAGE_EXECUTE_READ, as
 * kernelbase's WriteProcessMemory does for such a page but without its
 * cross-process notices of the protects, so that only the write itself is
 * between the two values. Returns the exit code. */
static int write_child(char **argv)
{
    DWORD pid = strtoul(argv[2], NULL, 0);
    BYTE *sleds = (BYTE *)(ULONG_PTR)strtoull(argv[3], NULL, 0);
    unsigned int range = strtoul(argv[4], NULL, 0);
    LONGLONG *times = (LONGLONG *)(ULONG_PTR)strtoull(argv[5], NULL, 0);
    LARGE_INTEGER before, after;
    LONGLONG pair[2];
    HANDLE process, turn, written, waits[2];
    char name[64];
    SIZE_T size;
    DWORD old;
    BYTE *addr;
    int i, value;

    process = OpenProcess(PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION | SYNCHRONIZE,
                          FALSE, pid);
    if (!process) return 2;
    snprintf(name, sizeof(name), "fex-smoke-turn-%lu", pid);
    if (!(turn = OpenEventA(SYNCHRONIZE, FALSE, name))) return 3;
    snprintf(name, sizeof(name), "fex-smoke-written-%lu", pid);
    if (!(written = OpenEventA(EVENT_MODIFY_STATE, FALSE, name))) return 3;
    waits[0] = turn;
    waits[1] = process;
    for (i = WRITE_RACE_SLEDS; i < WRITE_RACE_SLEDS + WRITE_RACE_ROUNDS; i++)
    {
        /* Ends with that process, which a failed round ends */
        if (WaitForMultipleObjects(2, waits, FALSE, 20000) != WAIT_OBJECT_0) return 4;
        spin_below(range);
        value = 2 * i + 1;
        addr = race_sled(sleds, i) + WRITE_RACE_MOV + 1;
        if (!VirtualProtectEx(process, addr, sizeof(value), PAGE_EXECUTE_READWRITE, &old)) return 7;
        QueryPerformanceCounter(&before);
        if (!WriteProcessMemory(process, addr, &value, sizeof(value), &size) || size != sizeof(value)) return 5;
        QueryPerformanceCounter(&after);
        if (!VirtualProtectEx(process, addr, sizeof(value), old, &old)) return 8;
        pair[0] = before.QuadPart;
        pair[1] = after.QuadPart;
        if (!WriteProcessMemory(process, times + 2 * (i - WRITE_RACE_SLEDS), pair, sizeof(pair), &size)
                || size != sizeof(pair))
            return 6;
        SetEvent(written);
    }
    return 0;
}

static int compare_times(const void *a, const void *b)
{
    LONGLONG x = *(const LONGLONG *)a, y = *(const LONGLONG *)b;

    return x < y ? -1 : x > y;
}

/* A child changes code on PAGE_EXECUTE_READ pages of this process with
 * WriteProcessMemory while this process compiles it, and the call after the
 * write must run what it wrote. The write takes no fault here, and the
 * instruction cache flush kernelbase queues for it drops the block once its
 * compile is done. Each round compiles a sled of its own, as the disk cache
 * would load the block of an earlier round instead. The child spins below what
 * half the first compiles of a sled took before it writes, and apc_sleeper runs
 * its protects meanwhile, so a write can land in the compile after it decoded
 * the mov. The disk cache must then not keep the block under the bytes as they
 * are after the write: the call after the write would load it and return the
 * old value. A write counts as landed in the call when it began after the call
 * began and ended before it returned; the child's protect comes first, so the
 * write is past the decode of the mov. With FEX_DISKCACHE=1 the case fails
 * when no write landed, as it then checked nothing. It prints how many did,
 * the compile time and what the writes took. */
static void cross_process_write_race(void)
{
    static LONGLONG times[2 * WRITE_RACE_ROUNDS], took[WRITE_RACE_ROUNDS];
    const int count = WRITE_RACE_SLEDS + WRITE_RACE_ROUNDS;
    BYTE *sleds = new_pages((SIZE_T)count * WRITE_RACE_SIZE, PAGE_READWRITE);
    const char *disk_cache = getenv("FEX_DISKCACHE"), *smc = getenv("FEX_SMCCHECKS");
    HANDLE waiter, turn, written, waits[2];
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi;
    char path[MAX_PATH], cmd[MAX_PATH + 128], name[64];
    double compile_time = 1e9, elapsed, ms;
    LARGE_INTEGER start, call_start, call_end, freq;
    volatile LONG done_sleeping = 0;
    unsigned int range;
    int i, k, r, in_call, landed = 0;
    DWORD code;

    for (i = 0; i < count; i++)
        race_fill(race_sled(sleds, i), 2 * i, smc && !strcmp(smc, "full") ? WRITE_RACE_ADDS_FULL : WRITE_RACE_ADDS);
    protect(sleds, (SIZE_T)count * WRITE_RACE_SIZE, PAGE_EXECUTE_READ);
    set_code(sleds, (SIZE_T)count * WRITE_RACE_SIZE);

    for (i = 0; i < WRITE_RACE_SLEDS; i++)
    {
        int (*fn)(void) = (void *)race_sled(sleds, i);

        QueryPerformanceCounter(&start);
        r = fn();
        elapsed = seconds_since(start);
        if (r != 2 * i) FAIL("sled %d returned %d, wanted %d", i, r, 2 * i);
        if (elapsed < compile_time) compile_time = elapsed;
    }
    QueryPerformanceCounter(&start);
    spin(1000000);
    range = (unsigned int)(compile_time / 2 / (seconds_since(start) / 1000000)) + 1;

    snprintf(name, sizeof(name), "fex-smoke-turn-%lu", GetCurrentProcessId());
    if (!(turn = CreateEventA(NULL, FALSE, FALSE, name))) FAIL("CreateEvent: %lu", GetLastError());
    snprintf(name, sizeof(name), "fex-smoke-written-%lu", GetCurrentProcessId());
    if (!(written = CreateEventA(NULL, FALSE, FALSE, name))) FAIL("CreateEvent: %lu", GetLastError());
    if (!(waiter = CreateThread(NULL, 0, apc_sleeper, (void *)&done_sleeping, 0, NULL)))
        FAIL("CreateThread: %lu", GetLastError());
    if (!GetModuleFileNameA(NULL, path, sizeof(path))) FAIL("GetModuleFileName: %lu", GetLastError());
    snprintf(cmd, sizeof(cmd), "\"%s\" write_child %lu %llu %u %llu", path, GetCurrentProcessId(),
             (unsigned long long)(ULONG_PTR)sleds, range, (unsigned long long)(ULONG_PTR)times);
    if (!CreateProcessA(path, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi))
        FAIL("CreateProcess: %lu", GetLastError());
    waits[0] = written;
    waits[1] = pi.hProcess;
    QueryPerformanceFrequency(&freq);
    ms = 1000.0 / freq.QuadPart;
    for (i = WRITE_RACE_SLEDS; i < count; i++)
    {
        int (*fn)(void) = (void *)race_sled(sleds, i);

        k = i - WRITE_RACE_SLEDS;
        QueryPerformanceCounter(&call_start);
        SetEvent(turn);
        r = fn();
        QueryPerformanceCounter(&call_end);
        if (WaitForMultipleObjects(2, waits, FALSE, 20000) != WAIT_OBJECT_0) FAIL("round %d: the child did not write", i);
        in_call = times[2 * k] > call_start.QuadPart && times[2 * k + 1] < call_end.QuadPart;
        landed += in_call;
        took[k] = times[2 * k + 1] - times[2 * k];
        compile_new_code();
        if ((r = fn()) != 2 * i + 1)
            FAIL("round %d: the call after the child's write returned %d, wanted %d; the write ran %.2f to %.2f ms"
                 " into a call of %.2f ms, %d of %d writes so far landed in the call", i, r, 2 * i + 1,
                 (times[2 * k] - call_start.QuadPart) * ms, (times[2 * k + 1] - call_start.QuadPart) * ms,
                 (call_end.QuadPart - call_start.QuadPart) * ms, landed, k + 1);
    }
    if (WaitForSingleObject(pi.hProcess, 20000)) FAIL("the child did not end");
    if (!GetExitCodeProcess(pi.hProcess, &code) || code) FAIL("the child exited with %lu", code);
    qsort(took, WRITE_RACE_ROUNDS, sizeof(took[0]), compare_times);
    printf("wine-build smoke test: %s cross_process_write_race %d of %d writes landed in the call, compile %.2f ms,"
           " writes took %.2f to %.2f ms, median %.2f\n", ARCH, landed, WRITE_RACE_ROUNDS, compile_time * 1000,
           took[0] * ms, took[WRITE_RACE_ROUNDS - 1] * ms, took[WRITE_RACE_ROUNDS / 2] * ms);
    if (!landed && disk_cache && !strcmp(disk_cache, "1")) FAIL("no write of the child landed in a call");
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    CloseHandle(turn);
    CloseHandle(written);
    done_sleeping = 1;
    if (!QueueUserAPC(end_sleep, waiter, 0)) FAIL("QueueUserAPC: %lu", GetLastError());
    WaitForSingleObject(waiter, INFINITE);
    CloseHandle(waiter);
}

/* movq (%eax), %mm0; paddd %mm0, %mm0; adds addps xmm1, xmm2, which take a
 * compile longer; mov %al, (%ecx); movq %mm0, (%edx); emms; ret. The same bytes
 * in both modes. Each sled gets its own count of addps, so that the disk cache
 * loads no block of another sled: in 64-bit code it leaves patchable
 * immediates out of the hash of the code, so an immediate cannot tell the sleds
 * apart, while a different length of code can. */
static void mmx_fill(BYTE *p, int adds)
{
    static const BYTE head[6] = {0x0f, 0x6f, 0x00, 0x0f, 0xfe, 0xc0};
    static const BYTE tail[8] = {0x88, 0x01, 0x0f, 0x7f, 0x02, 0x0f, 0x77, 0xc3};
    static const BYTE addps[3] = {0x0f, 0x58, 0xca};
    int i;

    memset(p, 0xcc, WRITE_RACE_SIZE);
    memcpy(p, head, sizeof(head));
    p += sizeof(head);
    for (i = 0; i < adds; i++, p += 3) memcpy(p, addps, 3);
    memcpy(p, tail, sizeof(tail));
}

static const LONGLONG mmx_in = 0x0000000300000005ll, mmx_want = 0x000000060000000all;

/* Calls a sled of mmx_fill with %mm0 cleared, which stores to *target and
 * returns its %mm0 in *result */
static void mmx_call(BYTE *code, BYTE *target, LONGLONG *result)
{
    asm volatile("pxor %%mm0, %%mm0\n\tcall *%[fn]"
                 : : [fn] "r"(code), "a"(&mmx_in), "c"(target), "d"(result) : "memory", "cc", "xmm1", "mm0");
}

static BYTE *volatile mmx_code;
static volatile LONGLONG mmx_start, mmx_end;

/* Calls mmx_code for each round, storing to a byte of its own, and keeps the
 * times of the last call */
static DWORD WINAPI mmx_caller(void *arg)
{
    LARGE_INTEGER start, end;
    LONGLONG result;
    BYTE byte;
    LONG seen = 0;

    while (!stop)
    {
        if (go == seen)
        {
            YieldProcessor();
            continue;
        }
        seen = go;
        QueryPerformanceCounter(&start);
        mmx_call(mmx_code, &byte, &result);
        QueryPerformanceCounter(&end);
        mmx_start = start.QuadPart;
        mmx_end = end.QuadPart;
        if (result != mmx_want) mmx_end = 0;
        done = seen;
    }
    return 0;
}

/* A PAGE_EXECUTE_READ sled that another thread compiles while this one makes it
 * PAGE_EXECUTE_READWRITE, placed by what a compile takes. The compile keeps the
 * code it generated for a read-only range, which keeps %mm0 in a host register
 * until the end of the block. Then this thread calls the sled again, now
 * writable, with a store into the sled's own page, which FEX handles by running
 * the block again from the store with the guest state of the context. The
 * disk cache must not have kept the first block under the writable protection:
 * the call would load it and return %mm0 as it was before the block. With
 * FEX_DISKCACHE=1 the case fails when no protect began during a compile, as it
 * then checked nothing. It prints how many did. */
static void rx_to_rwx_race(void)
{
    const int count = WRITE_RACE_SLEDS + WRITE_RACE_ROUNDS;
    BYTE *sleds = new_pages((SIZE_T)count * WRITE_RACE_SIZE, PAGE_READWRITE);
    const char *disk_cache = getenv("FEX_DISKCACHE"), *smc = getenv("FEX_SMCCHECKS");
    double compile_time = 1e9, elapsed, ms;
    LARGE_INTEGER start, protect_start, freq;
    LONGLONG result;
    unsigned int range;
    HANDLE thread;
    BYTE byte;
    int i, in_call, landed = 0;

    for (i = 0; i < count; i++)
        mmx_fill(race_sled(sleds, i), (smc && !strcmp(smc, "full") ? WRITE_RACE_ADDS_FULL : WRITE_RACE_ADDS) + i);
    protect(sleds, (SIZE_T)count * WRITE_RACE_SIZE, PAGE_EXECUTE_READ);
    set_code(sleds, (SIZE_T)count * WRITE_RACE_SIZE);
    for (i = 0; i < WRITE_RACE_SLEDS; i++)
    {
        QueryPerformanceCounter(&start);
        mmx_call(race_sled(sleds, i), &byte, &result);
        elapsed = seconds_since(start);
        if (result != mmx_want) FAIL("sled %d returned %#llx, wanted %#llx", i, result, mmx_want);
        if (elapsed < compile_time) compile_time = elapsed;
    }
    QueryPerformanceCounter(&start);
    spin(1000000);
    range = (unsigned int)(compile_time / (seconds_since(start) / 1000000)) + 1;

    QueryPerformanceFrequency(&freq);
    ms = 1000.0 / freq.QuadPart;
    if (!(thread = CreateThread(NULL, 0, mmx_caller, NULL, 0, NULL))) FAIL("CreateThread: %lu", GetLastError());
    for (i = WRITE_RACE_SLEDS; i < count; i++)
    {
        BYTE *sled = race_sled(sleds, i);

        mmx_code = sled;
        go = i;
        spin_below(range);
        QueryPerformanceCounter(&protect_start);
        protect(sled, WRITE_RACE_SIZE, PAGE_EXECUTE_READWRITE);
        while (done != i) YieldProcessor();
        if (!mmx_end) FAIL("round %d: the first call returned the wrong value", i);
        in_call = protect_start.QuadPart > mmx_start && protect_start.QuadPart < mmx_end;
        landed += in_call;
        FlushInstructionCache(GetCurrentProcess(), sled, WRITE_RACE_SIZE);
        mmx_call(sled, sled + WRITE_RACE_SIZE - 1, &result);
        if (result != mmx_want)
            FAIL("round %d: the call after the protect returned %#llx, wanted %#llx; the protect began %.2f ms"
                 " into a call of %.2f ms, %d protects so far began in the call", i, result, mmx_want,
                 (protect_start.QuadPart - mmx_start) * ms, (mmx_end - mmx_start) * ms, landed);
    }
    stop = 1;
    WaitForSingleObject(thread, INFINITE);
    CloseHandle(thread);
    printf("wine-build smoke test: %s rx_to_rwx_race %d of %d protects began in the call, compile %.2f ms\n",
           ARCH, landed, WRITE_RACE_ROUNDS, compile_time * 1000);
    if (!landed && disk_cache && !strcmp(disk_cache, "1")) FAIL("no protect began during a call");
}

/* The x87 cases compute 12 as (1 + 3) * 3 on the x87 stack, or load it, then
 * run an instruction that faults on a PAGE_NOACCESS page, which x87_handler
 * skips, and pop the 12. The instructions before the fault are in the same
 * block, so they have not written the x87 state back yet. Each case runs with
 * the precision control as found and with 24 bits, which FEX's reduced
 * precision runs in another copy of the block. The registers must be empty
 * again afterwards. */
static const float x87_three = 3.0f;
static const float x87_twelve_single = 12.0f;
static const double x87_twelve_double = 12.0;
static const LONGLONG x87_twelve_int = 12;
static volatile ULONG_PTR x87_bad;
static volatile LONG x87_faults;

/* In front of handler. The faulting instructions, fadds, fstps, fstpl, fistpl
 * and fistpll of (%eax), are two bytes long in both modes. */
static LONG CALLBACK x87_handler(EXCEPTION_POINTERS *e)
{
    EXCEPTION_RECORD *r = e->ExceptionRecord;
    const BYTE *pc = (const BYTE *)(ULONG_PTR)CTX_PC(e->ContextRecord);

    if (r->ExceptionCode != AV || r->NumberParameters < 2 || !x87_bad || r->ExceptionInformation[1] != x87_bad)
        return EXCEPTION_CONTINUE_SEARCH;
    /* An x87 instruction with the operand (%eax) */
    if ((pc[0] & 0xf8) != 0xd8 || (pc[1] & 0xc7) != 0)
        FAIL("fault reported at %p, which holds %02x %02x, not the x87 instruction", pc, pc[0], pc[1]);
    InterlockedIncrement(&x87_faults);
    CTX_PC(e->ContextRecord) += 2;
    return EXCEPTION_CONTINUE_EXECUTION;
}

#define X87_TWELVE "fld1\n\tfadds %[three]\n\tfmuls %[three]\n\t"
#define X87_OPERANDS                                                                                      \
    : [r] "=m"(r)                                                                                         \
    : [three] "m"(x87_three), [twelve_single] "m"(x87_twelve_single), [twelve_double] "m"(x87_twelve_double), \
      [twelve_int] "m"(x87_twelve_int), [bad] "a"(bad)                                                    \
    : "memory"

/* The pushes and the arithmetic before the fault are in its block */
static double __attribute__((noinline)) x87_mid_block(const float *bad)
{
    double r;

    asm volatile(X87_TWELVE "fadds (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

/* Two values on the stack at the fault, 4 from an instruction before the one
 * that pushed the 3 */
static double __attribute__((noinline)) x87_two_values(const float *bad)
{
    double r;

    asm volatile("fld1\n\tfadds %[three]\n\tflds %[three]\n\tfadds (%[bad])\n\tfmulp\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

/* The faulting instruction is the last use of the 12, as a 32-bit float, a
 * 64-bit float or an integer */
static double __attribute__((noinline)) x87_store_pop(const float *bad)
{
    double r;

    asm volatile(X87_TWELVE "fstps (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

static double __attribute__((noinline)) x87_store_pop_double(const float *bad)
{
    double r;

    asm volatile(X87_TWELVE "fstpl (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

static double __attribute__((noinline)) x87_store_pop_int(const float *bad)
{
    double r;

    asm volatile(X87_TWELVE "fistpl (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

/* The 12 is the operand of the load, which the store that faults would store
 * unchanged, as a 32-bit float, a 64-bit float or a 64-bit integer */
static double __attribute__((noinline)) x87_operand_single(const float *bad)
{
    double r;

    asm volatile("flds %[twelve_single]\n\tfstps (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

static double __attribute__((noinline)) x87_operand_double(const float *bad)
{
    double r;

    asm volatile("fldl %[twelve_double]\n\tfstpl (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

static double __attribute__((noinline)) x87_operand_int(const float *bad)
{
    double r;

    asm volatile("fildll %[twelve_int]\n\tfistpll (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

/* The fault starts a block, which works without FEX keeping a record */
static double __attribute__((noinline)) x87_block_start(const float *bad)
{
    double r;

    asm volatile(X87_TWELVE "jmp 1f\n1:\n\tfadds (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

/* The fmuls reads a register of the block before, which FEX does on its slow
 * path */
static double __attribute__((noinline)) x87_live_in(const float *bad)
{
    double r;

    asm volatile("fld1\n\tfadds %[three]\n\tjmp 1f\n1:\n\tfmuls %[three]\n\tfadds (%[bad])\n\tfstpl %[r]" X87_OPERANDS);
    return r;
}

static void x87_check(const char *what, double (*fn)(const float *), WORD fcw)
{
    BYTE area[512 + 16];
    BYTE *fx = (BYTE *)(((ULONG_PTR)area + 15) & ~(ULONG_PTR)15);
    WORD old_fcw, fsw;
    /* In memory before the fxsave, not in ST0 */
    volatile double r;

    x87_faults = 0;
    asm volatile("fnstcw %0\n\tfldcw %1" : "=m"(old_fcw) : "m"(fcw));
    r = fn((const float *)x87_bad);
    asm volatile("fxsave %0" : "=m"(*(BYTE (*)[512])fx));
    asm volatile("fldcw %0" : : "m"(old_fcw));
    memcpy(&fsw, fx + 2, sizeof(fsw));
    if (x87_faults != 1) FAIL("%s, FCW %#x: %ld faults, wanted 1", what, fcw, x87_faults);
    if (r != 12.0 || (fsw & 0x3800) || fx[4])
        FAIL("%s, FCW %#x: got %g with TOP %d and tags %#x, wanted 12 with TOP 0 and no tags", what, fcw, r,
             (fsw >> 11) & 7, fx[4]);
}

static void x87_fault(void)
{
    static const struct
    {
        const char *name;
        double (*fn)(const float *);
    } x87_cases[] =
    {
        {"fault in the middle of a block", x87_mid_block},
        {"fault with two values on the stack", x87_two_values},
        {"fault in a store that pops", x87_store_pop},
        {"fault in a 64-bit store that pops", x87_store_pop_double},
        {"fault in an integer store that pops", x87_store_pop_int},
        {"fault in a store of a loaded 32-bit float", x87_operand_single},
        {"fault in a store of a loaded 64-bit float", x87_operand_double},
        {"fault in a store of a loaded 64-bit integer", x87_operand_int},
        {"fault at the start of a block", x87_block_start},
        {"fault after a value from the block before", x87_live_in},
    };
    void *veh = AddVectoredExceptionHandler(1, x87_handler);
    WORD fcw;
    size_t i;

    x87_bad = (ULONG_PTR)new_pages(PAGE, PAGE_NOACCESS);
    asm volatile("fnstcw %0" : "=m"(fcw));
    for (i = 0; i < sizeof(x87_cases) / sizeof(x87_cases[0]); i++)
    {
        x87_check(x87_cases[i].name, x87_cases[i].fn, fcw);
        x87_check(x87_cases[i].name, x87_cases[i].fn, fcw & ~0x300);
    }
    x87_bad = 0;
    RemoveVectoredExceptionHandler(veh);
}

/* The same values in code on an RWX page that writes to its own page, which FEX
 * handles by running the block again from the store */
static void x87_inline_smc(void)
{
    static const BYTE code[] =
    {
        0xd9, 0xe8,       /* fld1 */
        0xd8, 0x40, 0x40, /* fadds 0x40(%eax) */
        0xd8, 0x48, 0x40, /* fmuls 0x40(%eax) */
        0x88, 0x40, 0x48, /* mov %al, 0x48(%eax) */
        0xdd, 0x19,       /* fstpl (%ecx) */
        0xc3,             /* ret */
    };
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    ULONG_PTR ax;
    double r, *rp;
    int i;

    memcpy(page, code, sizeof(code));
    memcpy(page + 0x40, &x87_three, sizeof(x87_three));
    set_code(page, PAGE);
    for (i = 0; i < 3; i++)
    {
        r = 0;
        rp = &r;
        ax = (ULONG_PTR)page;
        asm volatile("call *%[fn]\n\tfnstsw %%ax" : "+a"(ax), "+c"(rp) : [fn] "d"(page) : "memory");
        if (r != 12.0 || (ax & 0x3800)) FAIL("call %d: got %g with TOP %d, wanted 12 with TOP 0", i, r, (int)(ax >> 11) & 7);
    }
}

/* A push into the code's own page after a load and a store that pops it: the
 * x87 state at the push is the one the block started with, which FEX must keep
 * when it runs the block again from the push. */
static void x87_push_smc(void)
{
    static const BYTE code[] =
    {
        0xd9, 0x40, 0x40,             /* flds 0x40(%eax) */
        0xd9, 0x19,                   /* fstps (%ecx) */
#ifdef __x86_64__
        0x48, 0x89, 0xe2,             /* mov %rsp, %rdx */
        0x48, 0x8d, 0x60, 0x7c,       /* lea 0x7c(%rax), %rsp */
        0x51,                         /* push %rcx */
        0x48, 0x89, 0xd4,             /* mov %rdx, %rsp */
#else
        0x89, 0xe2,                   /* mov %esp, %edx */
        0x8d, 0x60, 0x7c,             /* lea 0x7c(%eax), %esp */
        0x51,                         /* push %ecx */
        0x89, 0xd4,                   /* mov %edx, %esp */
#endif
        0xc3,                         /* ret */
    };
    BYTE *page = new_pages(PAGE, PAGE_EXECUTE_READWRITE);
    BYTE area[512 + 16];
    BYTE *fx = (BYTE *)(((ULONG_PTR)area + 15) & ~(ULONG_PTR)15);
    ULONG_PTR ax, dx;
    float f, *fp;
    int i;

    memcpy(page, code, sizeof(code));
    memcpy(page + 0x40, &x87_three, sizeof(x87_three));
    set_code(page, PAGE);
    for (i = 0; i < 3; i++)
    {
        f = 0;
        fp = &f;
        ax = (ULONG_PTR)page;
        asm volatile("call *%[fn]\n\tfnstsw %%ax" : "+a"(ax), "+c"(fp), "=d"(dx) : [fn] "r"(page) : "memory");
        asm volatile("fxsave %0" : "=m"(*(BYTE (*)[512])fx));
        if (f != 3.0f || (ax & 0x3800) || fx[4])
            FAIL("call %d: stored %g with TOP %d and tags %#x, wanted 3 with TOP 0 and no tags", i, f, (int)(ax >> 11) & 7, fx[4]);
    }
}

/* x87_callee_saved: one x87 operation per function, between a marker in each
 * callee-saved register (x86_64: also all 128 bits of XMM6 to XMM15) and a
 * store of them, all in one block. FEX's helpers for these must leave the
 * guest's callee-saved registers alone. At full precision each operation calls
 * the F80 helper its name says (load_store, fldt and fstps_m32 the conversions
 * to f64 and f32, fistp_* and fisttp_* the integer ones, fbld and fbstp the BCD
 * ones), except fild_m32, whose conversion is inline and which is a control.
 * At reduced precision only fldt (the m80 conversion), fsin_big (the F64 sine
 * outside its inline range), fbld and fbstp still call a helper; fsincos uses
 * the inline sine and cosine. Every changed register is listed, then the case
 * fails. */
double x87_res __asm__("x87_res");
int x87_int __asm__("x87_int") = 3;
short x87_i16 __asm__("x87_i16");
long long x87_i64 __asm__("x87_i64");
unsigned char x87_bcd[10] __asm__("x87_bcd") = {0x42};
/* 1.0 as an 80-bit value: the significand's integer bit, then exponent 0x3fff */
const unsigned char x87_m80[10] __asm__("x87_m80") = {0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0x3f};
/* Past 2^23, where FEX's inline sine in reduced precision gives up */
const double x87_big __asm__("x87_big") = 1e9;

#if defined(__x86_64__)
#define X87_MEM(s) #s "(%rip)"
#define X87_XMM_IN(n, off) "movdqu x87_want+" #off "(%rip), %xmm" #n "\n"
#define X87_XMM_OUT(n, off) "movdqu %xmm" #n ", " #off "(%rax)\n"
#define X87_ENTER \
    "push %rbx\npush %rbp\npush %rsi\npush %rdi\npush %r12\npush %r13\npush %r14\npush %r15\n" \
    "sub $168, %rsp\n" \
    "movdqu %xmm6, 0(%rsp)\nmovdqu %xmm7, 16(%rsp)\nmovdqu %xmm8, 32(%rsp)\nmovdqu %xmm9, 48(%rsp)\n" \
    "movdqu %xmm10, 64(%rsp)\nmovdqu %xmm11, 80(%rsp)\nmovdqu %xmm12, 96(%rsp)\nmovdqu %xmm13, 112(%rsp)\n" \
    "movdqu %xmm14, 128(%rsp)\nmovdqu %xmm15, 144(%rsp)\nmov %rcx, 160(%rsp)\n" \
    "mov x87_want+0(%rip), %rbx\nmov x87_want+8(%rip), %rbp\nmov x87_want+16(%rip), %rsi\n" \
    "mov x87_want+24(%rip), %rdi\nmov x87_want+32(%rip), %r12\nmov x87_want+40(%rip), %r13\n" \
    "mov x87_want+48(%rip), %r14\nmov x87_want+56(%rip), %r15\n" \
    X87_XMM_IN(6, 64) X87_XMM_IN(7, 80) X87_XMM_IN(8, 96) X87_XMM_IN(9, 112) X87_XMM_IN(10, 128) \
    X87_XMM_IN(11, 144) X87_XMM_IN(12, 160) X87_XMM_IN(13, 176) X87_XMM_IN(14, 192) X87_XMM_IN(15, 208)
#define X87_LEAVE \
    "mov 160(%rsp), %rax\n" \
    "mov %rbx, 0(%rax)\nmov %rbp, 8(%rax)\nmov %rsi, 16(%rax)\nmov %rdi, 24(%rax)\n" \
    "mov %r12, 32(%rax)\nmov %r13, 40(%rax)\nmov %r14, 48(%rax)\nmov %r15, 56(%rax)\n" \
    X87_XMM_OUT(6, 64) X87_XMM_OUT(7, 80) X87_XMM_OUT(8, 96) X87_XMM_OUT(9, 112) X87_XMM_OUT(10, 128) \
    X87_XMM_OUT(11, 144) X87_XMM_OUT(12, 160) X87_XMM_OUT(13, 176) X87_XMM_OUT(14, 192) X87_XMM_OUT(15, 208) \
    "movdqu 0(%rsp), %xmm6\nmovdqu 16(%rsp), %xmm7\nmovdqu 32(%rsp), %xmm8\nmovdqu 48(%rsp), %xmm9\n" \
    "movdqu 64(%rsp), %xmm10\nmovdqu 80(%rsp), %xmm11\nmovdqu 96(%rsp), %xmm12\nmovdqu 112(%rsp), %xmm13\n" \
    "movdqu 128(%rsp), %xmm14\nmovdqu 144(%rsp), %xmm15\nadd $168, %rsp\n" \
    "pop %r15\npop %r14\npop %r13\npop %r12\npop %rdi\npop %rsi\npop %rbp\npop %rbx\nret\n"
typedef unsigned long long x87_reg;
#define X87_XMM_NAMES(n) "xmm" #n " low", "xmm" #n " high"
static const char *const x87_reg_names[] = {
    "rbx", "rbp", "rsi", "rdi", "r12", "r13", "r14", "r15",
    X87_XMM_NAMES(6), X87_XMM_NAMES(7), X87_XMM_NAMES(8), X87_XMM_NAMES(9), X87_XMM_NAMES(10),
    X87_XMM_NAMES(11), X87_XMM_NAMES(12), X87_XMM_NAMES(13), X87_XMM_NAMES(14), X87_XMM_NAMES(15),
};
#define X87_XMM_MARKS(n) 0xa0a0a0a0a0a0a000ull + n, 0xb0b0b0b0b0b0b000ull + n
const x87_reg x87_want[] __asm__("x87_want") = {
    0x1111111111111111ull, 0x2222222222222222ull, 0x3333333333333333ull, 0x4444444444444444ull,
    0x5555555555555555ull, 0x6666666666666666ull, 0x7777777777777777ull, 0x8888888888888888ull,
    X87_XMM_MARKS(6), X87_XMM_MARKS(7), X87_XMM_MARKS(8), X87_XMM_MARKS(9), X87_XMM_MARKS(10),
    X87_XMM_MARKS(11), X87_XMM_MARKS(12), X87_XMM_MARKS(13), X87_XMM_MARKS(14), X87_XMM_MARKS(15),
};
#else
#define X87_MEM(s) #s
#define X87_ENTER \
    "push %ebx\npush %ebp\npush %esi\npush %edi\nmov 20(%esp), %eax\npush %eax\n" \
    "mov x87_want+0, %ebx\nmov x87_want+4, %ebp\nmov x87_want+8, %esi\nmov x87_want+12, %edi\n"
#define X87_LEAVE \
    "mov (%esp), %eax\nmov %ebx, 0(%eax)\nmov %ebp, 4(%eax)\nmov %esi, 8(%eax)\nmov %edi, 12(%eax)\n" \
    "pop %eax\npop %edi\npop %esi\npop %ebp\npop %ebx\nret\n"
typedef unsigned int x87_reg;
static const char *const x87_reg_names[] = {"ebx", "ebp", "esi", "edi"};
const x87_reg x87_want[] __asm__("x87_want") = {0x11111111, 0x22222222, 0x33333333, 0x44444444};
#endif

#define X87_REGS (sizeof(x87_want) / sizeof(x87_want[0]))
#define X87_ST "fstpl " X87_MEM(x87_res) "\n"
#define X87_OP(name, body) \
    extern void x87op_##name(x87_reg *out) __asm__("x87op_" #name); \
    __asm__(".text\n.globl x87op_" #name "\n.p2align 4\nx87op_" #name ":\n" X87_ENTER body X87_LEAVE);

X87_OP(load_store, "fld1\n" X87_ST)
X87_OP(fldt, "fldt " X87_MEM(x87_m80) "\n" X87_ST)
X87_OP(fstps_m32, "fld1\nfstps " X87_MEM(x87_res) "\n")
X87_OP(fadd, "fld1\nfld1\nfaddp\n" X87_ST)
X87_OP(fsub, "fld1\nfld1\nfsubp\n" X87_ST)
X87_OP(fmul, "fld1\nfld1\nfmulp\n" X87_ST)
X87_OP(fdiv, "fld1\nfld1\nfdivrp\n" X87_ST)
X87_OP(fsqrt, "fld1\nfsqrt\n" X87_ST)
X87_OP(fcompp, "fld1\nfld1\nfcompp\n")
X87_OP(fsin, "fld1\nfsin\n" X87_ST)
X87_OP(fsin_big, "fldl " X87_MEM(x87_big) "\nfsin\n" X87_ST)
X87_OP(fcos, "fld1\nfcos\n" X87_ST)
X87_OP(fptan, "fld1\nfptan\n" X87_ST X87_ST)
X87_OP(fpatan, "fld1\nfld1\nfpatan\n" X87_ST)
X87_OP(fsincos, "fld1\nfsincos\n" X87_ST X87_ST)
X87_OP(f2xm1, "fld1\nf2xm1\n" X87_ST)
X87_OP(fyl2x, "fld1\nfld1\nfyl2x\n" X87_ST)
X87_OP(fyl2xp1, "fld1\nfldz\nfyl2xp1\n" X87_ST)
X87_OP(fscale, "fld1\nfld1\nfscale\n" X87_ST X87_ST)
X87_OP(fprem, "fld1\nfld1\nfprem\n" X87_ST X87_ST)
X87_OP(fprem1, "fld1\nfld1\nfprem1\n" X87_ST X87_ST)
X87_OP(frndint, "fld1\nfrndint\n" X87_ST)
X87_OP(fxtract, "fld1\nfxtract\n" X87_ST X87_ST)
X87_OP(fbstp, "fld1\nfbstp " X87_MEM(x87_bcd) "\n")
X87_OP(fbld, "fbld " X87_MEM(x87_bcd) "\n" X87_ST)
X87_OP(fistp_m16, "fld1\nfistps " X87_MEM(x87_i16) "\n")
X87_OP(fistp_m32, "fld1\nfistpl " X87_MEM(x87_int) "\n")
X87_OP(fistp_m64, "fld1\nfistpll " X87_MEM(x87_i64) "\n")
X87_OP(fisttp_m16, "fld1\nfisttps " X87_MEM(x87_i16) "\n")
X87_OP(fisttp_m32, "fld1\nfisttpl " X87_MEM(x87_int) "\n")
X87_OP(fisttp_m64, "fld1\nfisttpll " X87_MEM(x87_i64) "\n")
X87_OP(fild_m32, "fildl " X87_MEM(x87_int) "\n" X87_ST)

static void x87_callee_saved(void)
{
#define X87(name) { #name, x87op_##name }
    static const struct
    {
        const char *name;
        void (*run)(x87_reg *);
    } ops[] =
    {
        X87(load_store), X87(fldt), X87(fstps_m32), X87(fadd), X87(fsub), X87(fmul), X87(fdiv), X87(fsqrt),
        X87(fcompp), X87(fsin), X87(fsin_big), X87(fcos), X87(fptan), X87(fpatan), X87(fsincos), X87(f2xm1),
        X87(fyl2x), X87(fyl2xp1), X87(fscale), X87(fprem), X87(fprem1), X87(frndint), X87(fxtract),
        X87(fbstp), X87(fbld), X87(fistp_m16), X87(fistp_m32), X87(fistp_m64), X87(fisttp_m16),
        X87(fisttp_m32), X87(fisttp_m64), X87(fild_m32),
    };
#undef X87
    x87_reg out[X87_REGS];
    char changed[2048] = "";
    size_t i, r, len;

    for (i = 0; i < sizeof(ops) / sizeof(ops[0]); i++)
    {
        memset(out, 0, sizeof(out));
        ops[i].run(out);
        for (r = 0; r < X87_REGS; r++)
        {
            if (out[r] == x87_want[r]) continue;
            len = strlen(changed);
            snprintf(changed + len, sizeof(changed) - len, "%s %s %s=%#llx", len ? "," : "", ops[i].name,
                     x87_reg_names[r], (unsigned long long)out[r]);
        }
    }
    if (changed[0]) FAIL("changed:%s", changed);
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
    CASE(cross_process_write_race),
    CASE(rx_to_rwx_race),
    CASE(x87_fault),
    CASE(x87_inline_smc),
    CASE(x87_push_smc),
    CASE(x87_callee_saved),
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
    if (argc == 6 && !strcmp(argv[1], "child")) return child_protect(argv);
    if (argc == 6 && !strcmp(argv[1], "write_child")) return write_child(argv);
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
