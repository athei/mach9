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
 * puts the thread into such a call directly, in DuplicateHandle() and
 * CloseHandle(). The other cases get there through a second defect, which
 * is still open: a thread killed inside a call on the event, or while it
 * waits on it, keeps its reference on the handle's entry in ntdll's cache
 * of sync objects, and every call on a later handle with that value then
 * asks the wineserver under that mutex.
 *
 * All cases but create run in order, or just those named on the command
 * line. Each prints its name first. A lock left held shows as a hang, which
 * the caller's time limit ends; the last line then names the case. There is
 * no watchdog thread, as in fex.c.
 *
 * Without the unlock the native program hangs in pulse, wait, wait-two and
 * close with msync on, and in close alone with WINEMSYNC=0. The i686
 * and x86_64 programs fail in no case without it, so there they only guard
 * against a regression.
 *
 * While a thread can be terminated, no case may allocate or free on ntdll's
 * unix side with the server signals unblocked: a thread killed inside
 * malloc leaves a lock of libmalloc held, and the next thread that
 * allocates aborts the process (mach9 #147, #151). close therefore reaches
 * only NtDuplicateObject() and NtClose() (dlls/ntdll/unix/server.c). Both
 * take fd_cache_mutex in an uninterrupted section, which blocks those
 * signals. Without DUPLICATE_CLOSE_SOURCE, NtDuplicateObject() only sends
 * a dup_handle request in it. NtClose() calls remove_fd_from_cache() and
 * close_inproc_sync() (dlls/ntdll/unix/sync.c; the duplicate is never
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
 * Nor may a thread that can be terminated be the first of the process to
 * use an msync object on a page the process has not mapped: it maps the
 * page under shm_addrs_lock with the signals unblocked, and a kill there
 * leaves the lock held (mach9 #138). The main thread therefore resets each
 * event, those that stay open included, before a thread uses it, and the
 * thread of alertable gets its alert object, a new one, before it says
 * that it runs.
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

static volatile LONG started;
static HANDLE other_event, other_auto_event;

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
    volatile unsigned int spin;

    InterlockedExchange(&started, 1);
    for (;;)
    {
        if (DuplicateHandle(GetCurrentProcess(), arg, GetCurrentProcess(), &dup, 0, FALSE, DUPLICATE_SAME_ACCESS))
            CloseHandle(dup);
        /* without a pause between the calls a round of the x86_64 program
         * takes milliseconds, and the run up to seconds (mach9 #152) */
        for (spin = 4096; spin; spin--) YieldProcessor();
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

static DWORD WINAPI create_thread(void *arg)
{
    InterlockedExchange(&started, 1);
    for (;;) CloseHandle(CreateEventA(NULL, TRUE, FALSE, NULL));
    return 0;
}

static const struct
{
    const char *name;
    LPTHREAD_START_ROUTINE proc;
    int by_default;
    int check_pulse;  /* auto-reset event; checked with pulse_missed() after the kill */
} cases[] =
{
    { "pulse", pulse_thread, 1, 0 },
    { "wait", wait_thread, 1, 0 },
    { "wait-two", wait_two_thread, 1, 0 },
    { "close", close_thread, 1, 0 },
    { "alertable", alertable_thread, 1, 1 },
    { "two-auto", two_auto_thread, 1, 1 },
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

static int run_case(const char *name, LPTHREAD_START_ROUTINE proc, int check_pulse)
{
    HANDLE event, thread;
    volatile unsigned int spin;
    int i, tries;

    printf("wine-build smoke test: %s case %s\n", ARCH, name);
    fflush(stdout);

    for (i = 0; i < (check_pulse ? PULSE_ROUNDS : ROUNDS); i++)
    {
        if (!(event = CreateEventA(NULL, !check_pulse, FALSE, NULL)))
        {
            printf("wine-build smoke test: %s %s failed: CreateEvent error %lu\n", ARCH, name, GetLastError());
            return 0;
        }
        ResetEvent(event);

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
    return 1;
}

int main(int argc, char **argv)
{
    unsigned int i;
    int arg, ran = 0;

    other_event = CreateEventA(NULL, TRUE, FALSE, NULL);
    other_auto_event = CreateEventA(NULL, FALSE, FALSE, NULL);
    /* map their msync pages now: a thread killed while it maps one leaves
     * shm_addrs_lock held (mach9 #138) */
    ResetEvent(other_event);
    ResetEvent(other_auto_event);
    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        for (arg = 1; arg < argc; arg++)
            if (!strcmp(argv[arg], cases[i].name)) break;
        if (argc > 1 ? arg == argc : !cases[i].by_default) continue;
        if (!run_case(cases[i].name, cases[i].proc, cases[i].check_pulse)) return 1;
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
