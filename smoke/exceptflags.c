/* The program bundle-wine.sh builds for each x86 PE architecture and runs under
 * FEX, to check the EFLAGS around exception dispatch. x64 Windows runs the
 * dispatcher and the handlers with TF, DF and AC clear, hands them a context
 * with the EFLAGS of the faulting code, and a handler that continues that
 * context gets them all back. An x86_64 program runs as ARM64EC, where FEX and
 * Wine carry PF, AF and DF, which no ARM64 flag holds, in the context's Cpsr;
 * an i686 program gets them from wow64's 32-bit context. AC is not checked in
 * the handler: ARM64EC handlers run with the AC of the faulting code, since the
 * context has no bit left to give it back.
 *
 * Each check prints PASS or FAIL with its name; the names say what they check:
 *   veh-*         DF in a vectored handler (0) and in its context (1), and
 *                 after the handler fixed the fault and continued (1)
 *   rep-*         std; rep stosb running down into a no-access page that the
 *                 handler makes writable: the fill must finish downwards
 *   pfaf-continue PF and AF set before the fault survive a continue while the
 *                 handler computed other flags
 *   nested-*      a second fault inside the handler, continued, then the outer
 *                 one: DF in the handler stays 0, the outer code gets 1 back
 *   skip-*        the handler moves the instruction pointer past the faulting
 *                 store: the code there gets DF back
 *   tf-*          a TF single step with DF set: DF in the handler (0) and after
 *                 the continue (1), and TF cleared
 *   ac-*          AC across a fault: after the continue (1)
 *   seh-*         (x86_64) DF in a frame-based handler (0) and in the code
 *                 RtlUnwind resumes (0)
 *   stale         (x86_64) after that unwind, NtContinue to the unwound
 *                 faulting RIP and RSP with DF clear keeps DF clear
 * The exit code is 0 only when every check passed. */

#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <windows.h>

#if defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#else
#error "unknown architecture"
#endif

#define DF 0x400
#define TF 0x100
#define AC 0x40000

typedef uintptr_t reg;
static int fails, checks;
static uint8_t *page;  /* the first of its two pages has no access while armed */
static uint8_t *page2; /* no access while armed, for the nested fault */
static volatile LONG veh_df = -1, veh_ctx_df = -1, veh_hits, inner_df = -1;
static volatile int veh_mode;
static volatile reg skip_target;

#define MODE_FIX 0
#define MODE_NESTED 1
#define MODE_SKIP 2

#ifdef __x86_64__
#define PUSHF "pushfq"
#define POPF "popfq"
#define PTR_SP "rsp"
#else
#define PUSHF "pushfl"
#define POPF "popfl"
#define PTR_SP "esp"
#endif

static reg read_flags(void)
{
    reg f;

    __asm__ volatile(PUSHF "\n\tpop %0" : "=r"(f));
    return f;
}

static volatile int sink;

static LONG CALLBACK veh(EXCEPTION_POINTERS *ep)
{
    DWORD old;
    uint8_t *addr;
    reg fl = read_flags();

    if (ep->ExceptionRecord->ExceptionCode == EXCEPTION_SINGLE_STEP)
    {
        veh_df = !!(fl & DF);
        veh_hits++;
        return EXCEPTION_CONTINUE_EXECUTION;
    }
    if (ep->ExceptionRecord->ExceptionCode != EXCEPTION_ACCESS_VIOLATION) return EXCEPTION_CONTINUE_SEARCH;
    addr = (uint8_t *)ep->ExceptionRecord->ExceptionInformation[1];
    if (addr >= page2 && addr < page2 + 0x1000)
    {
        VirtualProtect(page2, 0x1000, PAGE_READWRITE, &old);
        return EXCEPTION_CONTINUE_EXECUTION;
    }
    if (addr < page || addr >= page + 0x1000) return EXCEPTION_CONTINUE_SEARCH;
    veh_df = !!(fl & DF);
    veh_ctx_df = !!(ep->ContextRecord->EFlags & DF);
    veh_hits++;
    sink = sink * 3 + 1;
    if (veh_mode == MODE_NESTED)
    {
        page2[0x10] = 1; /* faults, and the branch above continues it */
        inner_df = !!(read_flags() & DF);
    }
    VirtualProtect(page, 0x1000, PAGE_READWRITE, &old);
#ifdef __x86_64__
    if (veh_mode == MODE_SKIP) ep->ContextRecord->Rip = skip_target;
#else
    if (veh_mode == MODE_SKIP) ep->ContextRecord->Eip = skip_target;
#endif
    /* leave PF=0 and AF=0 in the handler's own flags */
    __asm__ volatile("mov $1,%%eax\n\tsub $0,%%eax" ::: "eax", "cc");
    return EXCEPTION_CONTINUE_EXECUTION;
}

static void check(const char *name, long got, long want)
{
    checks++;
    if (got != want) fails++;
    printf("wine-build smoke test: %s %s %-14s got %ld want %ld\n", ARCH, got == want ? "PASS" : "FAIL", name, got,
           want);
}

static void arm(int mode)
{
    DWORD old;

    memset(page, 0x55, 0x2000);
    memset(page2, 0x55, 0x1000);
    VirtualProtect(page, 0x1000, PAGE_NOACCESS, &old);
    VirtualProtect(page2, 0x1000, PAGE_NOACCESS, &old);
    veh_df = veh_ctx_df = inner_df = -1;
    veh_hits = 0;
    veh_mode = mode;
}

/* std; a store into the armed page; EFLAGS after it */
static reg store_df(uint8_t *p)
{
    reg fl;

    __asm__ volatile("std\n\tmovb $1,(%1)\n\t" PUSHF "\n\tpop %0\n\tcld" : "=r"(fl) : "r"(p) : "memory", "cc");
    return fl;
}

static void test_veh(void)
{
    void *h = AddVectoredExceptionHandler(1, veh);
    reg fl;

    arm(MODE_FIX);
    fl = store_df(page + 0x800);
    check("veh-hits", veh_hits, 1);
    check("veh-entry", veh_df, 0);
    check("veh-context", veh_ctx_df, 1);
    check("veh-continue", !!(fl & DF), 1);

    {
        reg di = (reg)(page + 0x1000 + 0x7ff), cx = 0x1000;
        size_t i, bad = 0;

        arm(MODE_FIX);
        __asm__ volatile("std\n\trep stosb\n\tcld" : "+D"(di), "+c"(cx) : "a"(0xab) : "memory", "cc");
        for (i = 0; i < 0x2000; i++)
            if (page[i] != ((i >= 0x800 && i < 0x1800) ? 0xab : 0x55)) bad++;
        check("rep-hits", veh_hits, 1);
        check("rep-bytes-bad", (long)bad, 0);
        check("rep-edi", (long)(di - (reg)(page + 0x7ff)), 0);
        check("rep-ecx", (long)cx, 0);
    }

    {
        reg in = 0x2 | 0x4 | 0x10, out; /* PF and AF */

        arm(MODE_FIX);
        __asm__ volatile("push %1\n\t" POPF "\n\tmovb $1,(%2)\n\t" PUSHF "\n\tpop %0"
                         : "=r"(out) : "r"(in), "r"(page + 0x800) : "memory", "cc");
        check("pfaf-hits", veh_hits, 1);
        check("pfaf-continue", (long)(out & 0x14), 0x14);
    }

    arm(MODE_NESTED);
    fl = store_df(page + 0x800);
    check("nested-hits", veh_hits, 1);
    check("nested-inner", inner_df, 0);
    check("nested-outer", !!(fl & DF), 1);

    arm(MODE_SKIP);
#ifdef __x86_64__
    __asm__ volatile("lea 2f(%%rip),%%rax\n\tmov %%rax,%2\n\t"
                     "std\n\tmovb $1,(%1)\n2:\n\tpushfq\n\tpop %0\n\tcld"
                     : "=r"(fl) : "r"(page + 0x800), "m"(skip_target) : "rax", "memory", "cc");
#else
    __asm__ volatile("movl $2f,%2\n\t"
                     "std\n\tmovb $1,(%1)\n2:\n\tpushfl\n\tpop %0\n\tcld"
                     : "=r"(fl) : "r"(page + 0x800), "m"(skip_target) : "memory", "cc");
#endif
    check("skip-hits", veh_hits, 1);
    check("skip-store", page[0x800], 0x55);
    check("skip-continue", !!(fl & DF), 1);

    arm(MODE_FIX);
    __asm__ volatile("std\n\t" PUSHF "\n\torl $0x100,(%%" PTR_SP ")\n\t" POPF "\n\tnop\n\t" PUSHF "\n\tpop %0\n\tcld"
                     : "=r"(fl) : : "memory", "cc");
    check("tf-hits", veh_hits, 1);
    check("tf-entry", veh_df, 0);
    check("tf-continue", !!(fl & DF), 1);
    check("tf-cleared", !!(fl & TF), 0);

    {
        reg out;

        arm(MODE_FIX);
        __asm__ volatile(PUSHF "\n\torl $0x40000,(%%" PTR_SP ")\n\t" POPF "\n\tmovb $1,(%1)\n\t" PUSHF "\n\tpop %0\n\t"
                         PUSHF "\n\tandl $~0x40000,(%%" PTR_SP ")\n\t" POPF
                         : "=r"(out) : "r"(page + 0x800) : "memory", "cc");
        check("ac-hits", veh_hits, 1);
        check("ac-continue", !!(out & AC), 1);
    }
    RemoveVectoredExceptionHandler(h);
}

#ifdef __x86_64__
static volatile LONG seh_df = -1;
static volatile reg seh_rsp;
EXCEPTION_DISPOSITION seh_handler(EXCEPTION_RECORD *rec, void *frame, CONTEXT *ctx, DISPATCHER_CONTEXT *dc);
EXCEPTION_DISPOSITION stale_handler(EXCEPTION_RECORD *rec, void *frame, CONTEXT *ctx, DISPATCHER_CONTEXT *dc);

/* seh_func(page) faults with DF set, and its handler unwinds to seh_target,
 * which returns the EFLAGS it runs with. stale_func(page) does the same at
 * stale_fault, unwinding to stale_target. stale_fault is entered again later
 * with NtContinue from stale_enter(ctx, NtContinue); there the store does not
 * fault, the EFLAGS go to stale_flags, and stale_landing returns to the caller
 * of stale_enter with its registers. */
__asm__(".text\n"
        ".globl seh_func\n"
        ".def seh_func; .scl 2; .type 32; .endef\n"
        ".seh_proc seh_func\n"
        "seh_func:\n"
        "  push %rbp\n"
        "  .seh_pushreg %rbp\n"
        "  mov %rsp,%rbp\n"
        "  .seh_setframe %rbp, 0\n"
        "  sub $32,%rsp\n"
        "  .seh_stackalloc 32\n"
        "  .seh_endprologue\n"
        "  .seh_handler seh_handler, @except\n"
        "  std\n"
        "  movb $1,(%rcx)\n"
        "  nop\n"
        "  cld\n"
        "  xor %eax,%eax\n"
        "  jmp 1f\n"
        ".globl seh_target\n"
        "seh_target:\n"
        "  pushfq\n"
        "  pop %rax\n"
        "  cld\n"
        "1:\n"
        "  add $32,%rsp\n"
        "  pop %rbp\n"
        "  ret\n"
        ".seh_endproc\n"

        ".globl stale_func\n"
        ".def stale_func; .scl 2; .type 32; .endef\n"
        ".seh_proc stale_func\n"
        "stale_func:\n"
        "  push %rbp\n"
        "  .seh_pushreg %rbp\n"
        "  mov %rsp,%rbp\n"
        "  .seh_setframe %rbp, 0\n"
        "  sub $32,%rsp\n"
        "  .seh_stackalloc 32\n"
        "  .seh_endprologue\n"
        "  .seh_handler stale_handler, @except\n"
        "  std\n"
        ".globl stale_fault\n"
        "stale_fault:\n"
        "  movb $1,(%rcx)\n"
        "  pushfq\n"
        "  pop %rax\n"
        "  mov %rax,stale_flags(%rip)\n"
        "  mov stale_saved_rsp(%rip),%rsp\n"
        "  jmp stale_landing\n"
        ".globl stale_target\n"
        "stale_target:\n"
        "  cld\n"
        "  xor %eax,%eax\n"
        "  add $32,%rsp\n"
        "  pop %rbp\n"
        "  ret\n"
        ".seh_endproc\n"

        ".globl stale_enter\n"
        "stale_enter:\n"
        /* what the continued context and Wine's dispatch below its RSP write
         * stays away from the saved registers */
        "  sub $4096,%rsp\n  orq $0,(%rsp)\n  sub $4096,%rsp\n  orq $0,(%rsp)\n"
        "  sub $4096,%rsp\n  orq $0,(%rsp)\n  sub $4096,%rsp\n  orq $0,(%rsp)\n"
        "  push %rbx\n  push %rbp\n  push %rsi\n  push %rdi\n  push %r12\n  push %r13\n  push %r14\n  push %r15\n"
        "  sub $168,%rsp\n"
        "  movdqu %xmm6,0(%rsp)\n  movdqu %xmm7,16(%rsp)\n  movdqu %xmm8,32(%rsp)\n  movdqu %xmm9,48(%rsp)\n"
        "  movdqu %xmm10,64(%rsp)\n  movdqu %xmm11,80(%rsp)\n  movdqu %xmm12,96(%rsp)\n  movdqu %xmm13,112(%rsp)\n"
        "  movdqu %xmm14,128(%rsp)\n  movdqu %xmm15,144(%rsp)\n"
        "  mov %rsp,stale_saved_rsp(%rip)\n"
        "  sub $40,%rsp\n"
        "  mov %rdx,%rax\n"
        "  xor %edx,%edx\n"
        "  call *%rax\n"
        "stale_landing:\n"
        "  movdqu 0(%rsp),%xmm6\n  movdqu 16(%rsp),%xmm7\n  movdqu 32(%rsp),%xmm8\n  movdqu 48(%rsp),%xmm9\n"
        "  movdqu 64(%rsp),%xmm10\n  movdqu 80(%rsp),%xmm11\n  movdqu 96(%rsp),%xmm12\n  movdqu 112(%rsp),%xmm13\n"
        "  movdqu 128(%rsp),%xmm14\n  movdqu 144(%rsp),%xmm15\n"
        "  add $168,%rsp\n"
        "  pop %r15\n  pop %r14\n  pop %r13\n  pop %r12\n  pop %rdi\n  pop %rsi\n  pop %rbp\n  pop %rbx\n"
        "  add $16384,%rsp\n"
        "  cld\n"
        "  ret\n");
extern reg seh_func(uint8_t *target);
extern reg stale_func(uint8_t *target);
extern void stale_enter(CONTEXT *ctx, void *ntcontinue);
extern char seh_target[], stale_target[], stale_fault[];
reg stale_flags, stale_saved_rsp;

EXCEPTION_DISPOSITION seh_handler(EXCEPTION_RECORD *rec, void *frame, CONTEXT *ctx, DISPATCHER_CONTEXT *dc)
{
    (void)ctx;
    (void)dc;
    if (rec->ExceptionFlags & (EXCEPTION_UNWINDING | EXCEPTION_EXIT_UNWIND)) return ExceptionContinueSearch;
    seh_df = !!(read_flags() & DF);
    RtlUnwind(frame, seh_target, rec, 0);
    return ExceptionContinueSearch;
}

EXCEPTION_DISPOSITION stale_handler(EXCEPTION_RECORD *rec, void *frame, CONTEXT *ctx, DISPATCHER_CONTEXT *dc)
{
    (void)dc;
    if (rec->ExceptionFlags & (EXCEPTION_UNWINDING | EXCEPTION_EXIT_UNWIND)) return ExceptionContinueSearch;
    seh_rsp = ctx->Rsp;
    RtlUnwind(frame, stale_target, rec, 0);
    return ExceptionContinueSearch;
}

static void test_seh(void)
{
    CONTEXT ctx;
    DWORD old;
    reg fl;

    memset(page, 0x55, 0x2000);
    VirtualProtect(page, 0x1000, PAGE_NOACCESS, &old);
    fl = seh_func(page + 0x800);
    VirtualProtect(page, 0x1000, PAGE_READWRITE, &old);
    check("seh-entry", seh_df, 0);
    check("seh-unwind", !!(fl & DF), 0);

    VirtualProtect(page, 0x1000, PAGE_NOACCESS, &old);
    stale_func(page + 0x800);
    VirtualProtect(page, 0x1000, PAGE_READWRITE, &old);
    memset(&ctx, 0, sizeof(ctx));
    ctx.ContextFlags = CONTEXT_FULL;
    RtlCaptureContext(&ctx);
    ctx.Rip = (reg)stale_fault;
    ctx.Rsp = seh_rsp;
    ctx.Rcx = (reg)(page + 0x800);
    ctx.EFlags &= ~DF;
    stale_flags = ~(reg)0;
    stale_enter(&ctx, (void *)GetProcAddress(GetModuleHandleA("ntdll.dll"), "NtContinue"));
    check("stale", !!(stale_flags & DF), 0);
}
#endif

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    page = VirtualAlloc(NULL, 0x2000, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    page2 = VirtualAlloc(NULL, 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!page || !page2)
    {
        printf("wine-build smoke test: %s VirtualAlloc failed: %lu\n", ARCH, GetLastError());
        return 1;
    }
    test_veh();
#ifdef __x86_64__
    test_seh();
#endif
    if (fails)
    {
        printf("wine-build smoke test: %s %d of %d exception flag checks failed\n", ARCH, fails, checks);
        return 1;
    }
    printf("wine-build smoke test: %s exception flags kept\n", ARCH);
    return 0;
}
