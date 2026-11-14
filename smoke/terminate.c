/* The program bundle-wine.sh builds for each PE architecture and runs under
 * the bundled wine to check that TerminateThread() does not leave a lock of
 * ntdll's unix side held that the rest of the process needs.
 *
 * Each case runs rounds that create an event, start a thread that uses it,
 * terminate the thread once it runs, wait for it and close both handles, so
 * that the next round's event gets the same handle value. The cases differ
 * in what the thread does when it is terminated:
 *
 *   pulse     calls PulseEvent() in a loop
 *   wait      waits on the event
 *   wait-two  waits on the event and on one that stays open
 *   close     creates and closes events of its own in a loop
 *
 * A thread that the wineserver kills while it is in a server call under
 * ntdll's fd_cache_mutex died with the mutex held, and the next thread that
 * closed a handle or used one that was not cached waited for ever. close
 * puts the thread into such a call directly, in CloseHandle(). The other
 * cases get there through a second defect, which is still open: a thread
 * killed inside a call on the event, or while it waits on it, keeps its
 * reference on the handle's entry in ntdll's cache of sync objects, and
 * every call on a later handle with that value then asks the wineserver
 * under that mutex.
 *
 * The cases run in order, or just those named on the command line. Each
 * prints its name first. The failure is a hang, which the caller's time
 * limit ends; the last line then names the case. There is no watchdog
 * thread, as in fex.c.
 *
 * Without the unlock the native program hangs in all four cases with msync
 * on, and in close alone with WINEMSYNC=0. The i686 and x86_64 programs
 * fail in no case without it, so there they only guard against a
 * regression.
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

static volatile LONG started;
static HANDLE other_event;

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
    InterlockedExchange(&started, 1);
    for (;;) CloseHandle(CreateEventA(NULL, TRUE, FALSE, NULL));
    return 0;
}

static const struct
{
    const char *name;
    LPTHREAD_START_ROUTINE proc;
} cases[] =
{
    { "pulse", pulse_thread },
    { "wait", wait_thread },
    { "wait-two", wait_two_thread },
    { "close", close_thread },
};

static unsigned int rng = 0x2545f491;

static unsigned int next_random(void)
{
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

static int run_case(const char *name, LPTHREAD_START_ROUTINE proc)
{
    HANDLE event, thread;
    volatile unsigned int spin;
    int i;

    printf("wine-build smoke test: %s case %s\n", ARCH, name);
    fflush(stdout);

    for (i = 0; i < ROUNDS; i++)
    {
        if (!(event = CreateEventA(NULL, TRUE, FALSE, NULL)))
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
        for (spin = next_random() % 20000; spin; spin--) YieldProcessor();

        TerminateThread(thread, 0);
        WaitForSingleObject(thread, INFINITE);
        CloseHandle(thread);
        CloseHandle(event);
    }
    return 1;
}

int main(int argc, char **argv)
{
    unsigned int i;
    int arg, ran = 0;

    other_event = CreateEventA(NULL, TRUE, FALSE, NULL);
    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        for (arg = 1; arg < argc; arg++)
            if (!strcmp(argv[arg], cases[i].name)) break;
        if (argc > 1 && arg == argc) continue;
        if (!run_case(cases[i].name, cases[i].proc)) return 1;
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
