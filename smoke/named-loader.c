/* Native filesystem/exec tests for the Darwin named loader lease. */
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <libproc.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "named_loader.h"

extern char **environ;

static void put_byte( const char *path, char byte )
{
    int fd = open( path, O_CREAT | O_WRONLY | O_TRUNC, 0600 );
    assert( fd != -1 && write( fd, &byte, 1 ) == 1 );
    close( fd );
}

static char profile_byte( int lease )
{
    int fd = openat( lease, "Game.app/Contents/embedded.provisionprofile", O_RDONLY );
    char byte;
    assert( fd != -1 && read( fd, &byte, 1 ) == 1 );
    close( fd );
    return byte;
}

int main( int argc, char **argv )
{
    char ntdll[PATH_MAX], profile[PATH_MAX], *new_exe, *old_exe, buf[16], byte;
    const char *tmp = argv[1];
    int new_lease, old_lease, pipefd[2], ready[2], status;
    pid_t pid;

    /* The bundle smoke checks the live executable, not the process's argv. */
    if (argc == 3 && !strcmp( argv[1], "--image" ))
    {
        char path[PROC_PIDPATHINFO_MAXSIZE];
        if (proc_pidpath( atoi( argv[2] ), path, sizeof(path) ) <= 0) return 1;
        puts( path );
        return 0;
    }

    if (argc == 4)  /* Came through exec into the named bundle. */
    {
        old_lease = atoi( argv[1] );
        assert( !(fcntl( old_lease, F_GETFD ) & FD_CLOEXEC) );
        adopt_named_loader();
        assert( fcntl( old_lease, F_GETFD ) == FD_CLOEXEC );
        assert( !getenv( NAMED_LOADER_LEASE_ENV ) );
        assert( write( atoi( argv[2] ), "!", 1 ) == 1 );
        assert( read( atoi( argv[3] ), &byte, 1 ) == 1 );
        _exit( 0 ); /* Don't rely on atexit cleanup. */
    }

    assert( argc == 2 );
    snprintf( ntdll, sizeof(ntdll), "%s/ntdll.so", tmp );
    snprintf( profile, sizeof(profile), "%s/wine.app/Contents/embedded.provisionprofile", tmp );
    put_byte( ntdll, 1 );
    put_byte( profile, 'A' );
    old_exe = prepare_named_loader( tmp, tmp, "Game", &old_lease );
    assert( old_exe && profile_byte( old_lease ) == 'A' );
    assert( !pipe( pipefd ) && !pipe( ready ) );
    pid = fork();
    assert( pid != -1 );
    if (!pid)
    {
        posix_spawnattr_t attr;
        char release[16], acquired[16];
        char *args[] = {old_exe, buf, acquired, release, NULL};
        close( pipefd[1] );
        close( ready[0] );
        assert( inherit_named_loader( old_lease, old_exe, tmp ) );
        snprintf( buf, sizeof(buf), "%d", old_lease );
        snprintf( acquired, sizeof(acquired), "%d", ready[1] );
        snprintf( release, sizeof(release), "%d", pipefd[0] );
        assert( !posix_spawnattr_init( &attr ) );
        assert( !posix_spawnattr_setflags( &attr, POSIX_SPAWN_SETEXEC ) );
        posix_spawn( NULL, old_exe, NULL, &attr, args, environ );
        _exit( 1 );
    }
    close( old_lease );
    close( pipefd[0] );
    close( ready[1] );
    assert( read( ready[0], &byte, 1 ) == 1 );
    close( ready[0] );

    /* A profile update with no executable change must still take effect;
     * pruning in a concurrent launch must not remove the running executable. */
    put_byte( profile, 'B' );
    new_exe = prepare_named_loader( tmp, tmp, "Game", &new_lease );
    assert( new_exe && strcmp( new_exe, old_exe ) && profile_byte( new_lease ) == 'B' );
    assert( !access( old_exe, X_OK ) );

    /* Neither a missing nor a redirected ntdll dependency may be selected. */
    assert( !unlinkat( new_lease, "ntdll.so", 0 ) );
    assert( !inherit_named_loader( new_lease, new_exe, tmp ) );
    assert( !symlinkat( profile, new_lease, "ntdll.so" ) );
    assert( !inherit_named_loader( new_lease, new_exe, tmp ) );
    close( new_lease );
    assert( write( pipefd[1], "!", 1 ) == 1 );
    close( pipefd[1] );
    assert( waitpid( pid, &status, 0 ) == pid && WIFEXITED( status ) && !WEXITSTATUS( status ) );

    /* A subsequent preparation reclaims both the dead and damaged entries. */
    {
        char *third = prepare_named_loader( tmp, tmp, "Game", &new_lease );
        assert( third && access( old_exe, F_OK ) && errno == ENOENT );
        assert( access( new_exe, F_OK ) && errno == ENOENT );
        assert( profile_byte( new_lease ) == 'B' );
        assert( inherit_named_loader( new_lease, third, tmp ) );
        close( new_lease );
        free( third );
    }
    free( new_exe );
    free( old_exe );
    puts( "named-loader: passed" );
    return 0;
}
