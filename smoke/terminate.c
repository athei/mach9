/* The program bundle-wine.sh builds for each PE architecture and runs under
 * the bundled wine to check that TerminateThread() does not leave a lock of
 * ntdll's unix side held that the rest of the process needs, nor a wait
 * registered with msync.
 *
 * Each case runs rounds that create an event, start a thread that uses it,
 * terminate the thread once it runs, wait for it and close both handles, so
 * that the next round's event gets the same handle value. The cases differ
 * in what the thread does when it is terminated:
 *
 *   pulse      calls PulseEvent() in a loop
 *   wait       waits on the event
 *   wait-two   waits on the event and on one that stays open
 *   close      duplicates the event and closes the duplicate in a loop
 *   alertable  waits alertably on the event, an auto-reset one
 *   two-auto   waits on the event, an auto-reset one, and on one that
 *              stays open
 *   new-page   resets events in a loop, each on an msync page that the
 *              process has not mapped
 *   query      calls VirtualQuery() on a view of reserved pages in a loop
 *   create     creates and closes events of its own in a loop; runs only
 *              when named
 *
 * msync registers a wait on several objects, and an alertable wait, with
 * the wineserver and raises a waiter count in each object's shared memory.
 * A thread killed in such a wait left both behind (mach9 #132). A
 * PulseEvent() of an auto-reset event then counted the dead thread as a
 * waiter and kept its pulse for a thread whose wait started before the
 * pulse but was not asleep on the event when it came. On Windows such a
 * thread, one suspended in its wait for example, misses the pulse. In
 * alertable and two-auto the main thread therefore lets the thread fall
 * asleep in its wait before it terminates it. Then a second thread waits
 * on the event alone, and the main thread suspends it, pulses the event
 * and resumes it: its wait must not return. The kill is asynchronous: the
 * wait for the terminated thread ends when the wineserver marks it dead,
 * before the thread has handled the signal and taken its wait back, so a
 * round counts as failed only when the check still fails after a few
 * pauses. Without the fix the waiter count stays raised for good, and both
 * cases fail in their first round with msync on, on every architecture.
 *
 * A thread that the wineserver kills while it is in a server call under
 * ntdll's fd_cache_mutex died with the mutex held, and the next thread that
 * closed a handle or used one that was not cached waited for ever. close
 * puts the thread into such a call directly, in CloseHandle(). The other
 * cases got there through a second defect: a thread killed inside a call on
 * the event, or while it waits on it, keeps its reference on the handle's
 * entry in ntdll's cache of sync objects (mach9 #137). Every call on a later
 * handle with that value asked the wineserver under that mutex; now such a
 * handle is cached in a spare entry. pulse checks that: from its second
 * round on, the main thread times ResetEvent() on the round's event against
 * the event that stays open, and a round counts as uncached when the
 * fastest of three batches is over 12 times as slow and over 3 us per call.
 * Measured per round: an uncached call costs 25 to 3000 times a cached one
 * (9 us and more), a cached one at most 3.5 times under FEX and 13 times
 * natively, but at most 2.3 us under FEX and 0.2 us natively, with 8 copies
 * at once, also in the background. A machine that is slower slows both
 * events alike, so the ratio holds. The case fails when more than a quarter
 * of the 300 rounds are uncached; without the spare entries nearly all of
 * them are.
 *
 * All cases but create run in order, or just those named on the command
 * line. Each prints its name first. A lock left held shows as a hang, which
 * the caller's time limit ends; the last line then names the case. There is
 * no watchdog thread, as in fex.c.
 *
 * Without the unlock of fd_cache_mutex the native program hangs in close,
 * with msync on and with WINEMSYNC=0, and without the spare entries also in
 * pulse, wait and wait-two with msync on. The i686 and x86_64 programs fail
 * in no case without it, so there they only guard against a regression.
 *
 * While a thread can be terminated, no case may allocate or free on ntdll's
 * unix side with the server signals unblocked: a thread killed inside
 * malloc leaves a lock of libmalloc held, and the next thread that
 * allocates aborts the process (mach9 #147, #151). close therefore reaches
 * only NtDuplicateObject() and NtClose() (dlls/ntdll/unix/server.c).
 * Without DUPLICATE_CLOSE_SOURCE, NtDuplicateObject() only sends a
 * dup_handle request, with those signals blocked. NtClose() takes
 * fd_cache_mutex in an uninterrupted section, which blocks them too, and
 * in it calls remove_fd_from_cache() and close_inproc_sync()
 * (dlls/ntdll/unix/sync.c; the duplicate is never
 * cached, and a release_inproc_sync() that drops the last reference calls
 * msync_close(), which builds its message on the stack) and sends a
 * close_handle request; none of them allocates. Outside the section the
 * thread runs kernelbase's DuplicateHandle() and CloseHandle(), which
 * allocate nothing either. create keeps the old shape of close for mach9
 * #138: NtCreateEvent() allocates the object attributes with malloc while
 * the signals are unblocked (wine_server_alloc_object_attributes()), so a
 * kill there aborts the process now and then. The bundle step does not
 * run it.
 *
 * A thread killed in a server call under ntdll's virtual_mutex died with
 * that mutex held in the same way, and every later allocation, mapping or
 * protection change of the process waited for ever (mach9 #138).
 * VirtualQuery() of a view of a section of reserved pages asks the
 * wineserver for the committed ranges under the mutex, and allocates
 * nothing, so query puts its thread there. Only the native program can
 * fail in it: FEX suspends a thread before it is killed, and the suspend
 * signal waits until the thread leaves the section, so in the i686 and
 * x86_64 programs query only guards against a regression.
 *
 * The first thread of a process that uses an msync object on a page the
 * process has not mapped maps the page under ntdll's shm_addrs_lock. That
 * lock was taken with the signals unblocked, and a thread killed with it
 * held left it held: the next thread that mapped a page aborted the process
 * (mach9 #138). new-page puts its thread there, on one new page after the
 * other, for as many rounds as it has pages; once they are used up, its
 * thread sleeps. In the other cases the main thread resets each event,
 * those that stay open included, before a thread uses it, and the thread
 * of alertable gets its alert object, a new one, before it says that it
 * runs, so that they check only what they are there for.
 *
 * The threads say when they run before they are terminated: terminating a
 * thread that has just started can leave the loader lock of the x86 code
 * held under FEX, which is not what this program checks. */

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

#define ROUNDS 300
/* the rounds of the cases that sleep before the kill and check a pulse */
#define PULSE_ROUNDS 10
/* the calls of a batch of reset_ticks() */
#define RESET_CALLS 64
/* an msync slot takes 16 bytes of a 16K page */
#define PAGE_SLOTS 1024
#define NEW_PAGES 32

static volatile LONG started;
static HANDLE other_event, other_auto_event;
static HANDLE fillers[NEW_PAGES * PAGE_SLOTS];
static volatile LONG next_page;
static void *reserve_view;

static DWORD WINAPI pulse_thread(void *arg)
{
    InterlockedExchange(&started, 1);
    for (;;) PulseEvent(arg);
    return 0;
}

static DWORD WINAPI wait_thread(void *arg)
{
    InterlockedExchange(&started, 1);
    for (;;) WaitForSingleObject(arg, INFINITE);
    return 0;
}

static DWORD WINAPI wait_two_thread(void *arg)
{
    HANDLE handles[2] = { arg, other_event };

    InterlockedExchange(&started, 1);
    for (;;) WaitForMultipleObjects(2, handles, FALSE, INFINITE);
    return 0;
}

static DWORD WINAPI close_thread(void *arg)
{
    HANDLE dup;

    InterlockedExchange(&started, 1);
    for (;;)
    {
        if (DuplicateHandle(GetCurrentProcess(), arg, GetCurrentProcess(), &dup, 0, FALSE, DUPLICATE_SAME_ACCESS))
            CloseHandle(dup);
    }
    return 0;
}

static DWORD WINAPI alertable_thread(void *arg)
{
    /* gets the thread's alert object and maps its page */
    WaitForSingleObjectEx(arg, 0, TRUE);
    InterlockedExchange(&started, 1);
    for (;;) WaitForSingleObjectEx(arg, INFINITE, TRUE);
    return 0;
}

static DWORD WINAPI two_auto_thread(void *arg)
{
    HANDLE handles[2] = { arg, other_auto_event };

    InterlockedExchange(&started, 1);
    for (;;) WaitForMultipleObjects(2, handles, FALSE, INFINITE);
    return 0;
}

static DWORD WINAPI new_page_thread(void *arg)
{
    LONG page;

    InterlockedExchange(&started, 1);
    while ((page = InterlockedIncrement(&next_page) - 1) < NEW_PAGES)
        ResetEvent(fillers[page * PAGE_SLOTS]);
    for (;;) Sleep(INFINITE);
    return 0;
}

static DWORD WINAPI query_thread(void *arg)
{
    MEMORY_BASIC_INFORMATION info;

    InterlockedExchange(&started, 1);
    for (;;) VirtualQuery(reserve_view, &info, sizeof(info));
    return 0;
}

static DWORD WINAPI create_thread(void *arg)
{
    InterlockedExchange(&started, 1);
    for (;;) CloseHandle(CreateEventA(NULL, TRUE, FALSE, NULL));
    return 0;
}

/* Events that no thread uses before new-page, PAGE_SLOTS for each of its
 * pages. The wineserver gives a new object the lowest free msync slot, so
 * fillers[i * PAGE_SLOTS] are on pages of their own, which the process has
 * not mapped. They stay open until the process exits. */
static int create_fillers(void)
{
    unsigned int i;

    for (i = 0; i < NEW_PAGES * PAGE_SLOTS; i++)
        if (!(fillers[i] = CreateEventA(NULL, TRUE, FALSE, NULL))) return 0;
    return 1;
}

/* a view of a section of reserved pages, whose committed ranges only the
 * wineserver knows */
static int map_reserve_view(void)
{
    HANDLE section;

    if (!(section = CreateFileMappingA(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE | SEC_RESERVE, 0, 0x10000, NULL)))
        return 0;
    reserve_view = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 0);
    CloseHandle(section);
    return reserve_view != NULL;
}

static const struct
{
    const char *name;
    LPTHREAD_START_ROUTINE proc;
    int by_default;
    int check_pulse;  /* auto-reset event; checked with pulse_missed() after the kill */
    int (*prepare)(void);
    int rounds;  /* 0 for the default */
    int check_cached;  /* the event of each round is cached; checked with reset_uncached() */
} cases[] =
{
    { "pulse", pulse_thread, 1, 0, .check_cached = 1 },
    { "wait", wait_thread, 1, 0 },
    { "wait-two", wait_two_thread, 1, 0 },
    { "close", close_thread, 1, 0 },
    { "alertable", alertable_thread, 1, 1 },
    { "two-auto", two_auto_thread, 1, 1 },
    { "new-page", new_page_thread, 1, 0, create_fillers, NEW_PAGES },
    { "query", query_thread, 1, 0, map_reserve_view },
    { "create", create_thread, 0, 0 },
};

static unsigned int rng = 0x2545f491;

static unsigned int next_random(void)
{
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

static DWORD WINAPI single_wait_thread(void *arg)
{
    InterlockedExchange(&started, 1);
    WaitForSingleObject(arg, INFINITE);
    return 0;
}

/* whether a thread suspended in its wait on the auto-reset event misses a
 * pulse, as on Windows */
static int pulse_missed(HANDLE event)
{
    CONTEXT context;
    HANDLE thread;
    DWORD ret;

    /* a waiter of an earlier try may have taken the pulse, and the set
     * that ended it then stayed */
    ResetEvent(event);
    started = 0;
    if (!(thread = CreateThread(NULL, 0, single_wait_thread, event, 0, NULL))) return 0;
    while (!started) YieldProcessor();
    Sleep(1);
    SuspendThread(thread);
    /* returns once the thread is suspended */
    context.ContextFlags = CONTEXT_CONTROL;
    GetThreadContext(thread, &context);
    PulseEvent(event);
    ResumeThread(thread);
    ret = WaitForSingleObject(thread, 20);
    SetEvent(event);
    WaitForSingleObject(thread, INFINITE);
    CloseHandle(thread);
    return ret == WAIT_TIMEOUT;
}

/* the fastest of a few batches of ResetEvent() calls on the event, in
 * performance counter ticks */
static LONGLONG reset_ticks(HANDLE event)
{
    LARGE_INTEGER start, end;
    LONGLONG best = -1;
    int batch, n;

    for (batch = 0; batch < 3; batch++)
    {
        QueryPerformanceCounter(&start);
        for (n = 0; n < RESET_CALLS; n++) ResetEvent(event);
        QueryPerformanceCounter(&end);
        if (best < 0 || end.QuadPart - start.QuadPart < best) best = end.QuadPart - start.QuadPart;
    }
    return best;
}

/* whether calls on the event, whose handle has the value of the one of the
 * round before, ask the wineserver: over 12 times as slow as on the event
 * that stays open, and over 3 us per call */
struct reset_stats
{
    double min_ratio, max_ratio;  /* of the reused handle value against the open event */
    double min_us, max_us;        /* per call on the reused handle value */
};

static int reset_uncached(HANDLE event, struct reset_stats *stats)
{
    LARGE_INTEGER freq;
    LONGLONG ticks = reset_ticks(event), ref = reset_ticks(other_event);
    double ratio, us;

    QueryPerformanceFrequency(&freq);
    ratio = (double)ticks / (ref ? ref : 1);
    us = ticks * 1e6 / freq.QuadPart / RESET_CALLS;
    if (!stats->max_us || ratio < stats->min_ratio) stats->min_ratio = ratio;
    if (!stats->max_us || us < stats->min_us) stats->min_us = us;
    if (ratio > stats->max_ratio) stats->max_ratio = ratio;
    if (us > stats->max_us) stats->max_us = us;
    /* Natively a batch of cached calls lasts about 10 ticks of the counter,
     * so the ratio of a cached round reaches 13 at times: there only the
     * 3 us floor tells it from an uncached one (0.2 us against 9 us and
     * more, 0.8 us on a machine 4 times as slow). */
    return ticks > 12 * ref && ticks * 1000000 > 3 * RESET_CALLS * freq.QuadPart;
}

static int run_case(const char *name, LPTHREAD_START_ROUTINE proc, int check_pulse,
                    int (*prepare)(void), int rounds, int check_cached)
{
    HANDLE event, thread;
    volatile unsigned int spin;
    struct reset_stats stats = { 0 };
    int i, tries, uncached = 0;

    printf("wine-build smoke test: %s case %s\n", ARCH, name);
    fflush(stdout);

    if (prepare && !prepare())
    {
        printf("wine-build smoke test: %s %s failed: setup error %lu\n", ARCH, name, GetLastError());
        return 0;
    }
    if (!rounds) rounds = check_pulse ? PULSE_ROUNDS : ROUNDS;
    for (i = 0; i < rounds; i++)
    {
        if (!(event = CreateEventA(NULL, !check_pulse, FALSE, NULL)))
        {
            printf("wine-build smoke test: %s %s failed: CreateEvent error %lu\n", ARCH, name, GetLastError());
            return 0;
        }
        ResetEvent(event);
        if (check_cached && i) uncached += reset_uncached(event, &stats);

        started = 0;
        if (!(thread = CreateThread(NULL, 0, proc, event, 0, NULL)))
        {
            printf("wine-build smoke test: %s %s failed: CreateThread error %lu\n", ARCH, name, GetLastError());
            return 0;
        }
        while (!started) YieldProcessor();
        /* the wait is registered within a few microseconds; a kill while it
         * registers can still leave it behind */
        if (check_pulse) Sleep(1);
        else for (spin = next_random() % 20000; spin; spin--) YieldProcessor();

        TerminateThread(thread, 0);
        WaitForSingleObject(thread, INFINITE);
        CloseHandle(thread);
        for (tries = 0; check_pulse && tries < 5 && !pulse_missed(event); tries++) Sleep(50);
        if (check_pulse && tries == 5)
        {
            printf("wine-build smoke test: %s %s failed: round %d: a suspended waiter got a pulse\n", ARCH, name, i);
            return 0;
        }
        CloseHandle(event);
    }
    if (check_cached)
        printf("wine-build smoke test: %s %s: %d of %d rounds uncached; reused handle value "
               "%.2f to %.2f us per call, %.1f to %.1f times the open event\n", ARCH, name, uncached, rounds - 1,
               stats.min_us, stats.max_us, stats.min_ratio, stats.max_ratio);
    if (uncached > rounds / 4)
    {
        printf("wine-build smoke test: %s %s failed: calls on a reused handle value asked the wineserver in %d of %d rounds\n",
               ARCH, name, uncached, rounds - 1);
        return 0;
    }
    return 1;
}

int main(int argc, char **argv)
{
    unsigned int i;
    int arg, ran = 0;

    other_event = CreateEventA(NULL, TRUE, FALSE, NULL);
    other_auto_event = CreateEventA(NULL, FALSE, FALSE, NULL);
    /* map their msync pages now; only new-page has its thread map pages */
    ResetEvent(other_event);
    ResetEvent(other_auto_event);
    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        for (arg = 1; arg < argc; arg++)
            if (!strcmp(argv[arg], cases[i].name)) break;
        if (argc > 1 ? arg == argc : !cases[i].by_default) continue;
        if (!run_case(cases[i].name, cases[i].proc, cases[i].check_pulse, cases[i].prepare, cases[i].rounds,
                      cases[i].check_cached))
            return 1;
        ran++;
    }
    if (!ran)
    {
        printf("wine-build smoke test: %s no such case\n", ARCH);
        return 1;
    }
    printf("wine-build smoke test: %s TerminateThread cases passed\n", ARCH);
    return 0;
}
