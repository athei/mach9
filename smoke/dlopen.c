/* The host program bundle-wine.sh builds to load the bundle's own dylibs
 * (lib/external) the way Wine's unix modules do, with dlopen. It loads each
 * path it is given with every dependency bound at once, prints dyld's
 * reason for each one that does not load, and exits with 1 if any did not. */

#include <dlfcn.h>
#include <stdio.h>

int main(int argc, char **argv)
{
    int i, failed = 0;

    for (i = 1; i < argc; i++)
    {
        if (!dlopen(argv[i], RTLD_NOW | RTLD_LOCAL))
        {
            fprintf(stderr, "  ERROR: %s\n", dlerror());
            failed = 1;
        }
    }
    return failed;
}
