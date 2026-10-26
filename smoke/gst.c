/* The host program bundle-wine.sh builds to check the bundle's GStreamer
 * outside Wine. It opens the libgstreamer-1.0 it is given (lib/external of
 * the bundle), initializes GStreamer the way winegstreamer.so does, without
 * forking a plugin scanner, and requires every plugin file in the directory
 * GST_PLUGIN_SYSTEM_PATH_1_0 names to be in the registry and to load: a
 * plugin whose initialization fails is left out. It prints the file name of
 * each, creates every element named on its command line, then prints the
 * path of each GStreamer, glib, FFmpeg, pcre2, libffi and libintl library
 * the process loaded from outside /usr/lib and /System, so the caller can
 * check that they all came from the bundle. The caller sets
 * GST_PLUGIN_SYSTEM_PATH_1_0 and GST_REGISTRY_1_0 as ntdll does for Wine,
 * the registry to a path of its own, so GStreamer scans the bundle's plugins
 * in this process. The exit status names the step that failed, after a line
 * saying why.
 *
 * The few GStreamer declarations it needs are written out here, so it builds
 * without GStreamer's headers. */

#include <dirent.h>
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct
{
    unsigned int domain;
    int code;
    char *message;
} GError;

typedef struct GList
{
    void *data;
    struct GList *next, *prev;
} GList;

typedef int (*init_check_t)(int *, char ***, GError **);
typedef void *(*factory_make_t)(const char *, const char *);
typedef void (*object_unref_t)(void *);
typedef char *(*version_string_t)(void);
typedef void *(*registry_get_t)(void);
typedef GList *(*registry_get_plugin_list_t)(void *);
typedef const char *(*plugin_get_filename_t)(void *);
typedef void *(*plugin_load_t)(void *);
typedef void (*plugin_list_free_t)(GList *);

enum
{
    FAIL_DLOPEN = 1,
    FAIL_SYMBOLS,
    FAIL_INIT,
    FAIL_PLUGIN,
    FAIL_ELEMENT,
};

/* The file name prefixes of the libraries the bundle's GStreamer brings. */
static const char *const prefixes[] =
{
    "libgst", "libglib", "libgobject", "libgmodule", "libgio", "libgthread",
    "libav", "libswresample", "libswscale", "libpcre", "libffi", "libintl",
};

static const char *base_name(const char *path)
{
    const char *base = strrchr(path, '/');
    return base ? base + 1 : path;
}

int main(int argc, char **argv)
{
    char *init_argv[] = { "gst", "--gst-disable-registry-fork", NULL };
    char **init_args = init_argv;
    int init_argc = 2, failed_plugin = 0, failed_element = 0, i;
    init_check_t init_check;
    factory_make_t factory_make;
    object_unref_t object_unref;
    version_string_t version_string;
    registry_get_t registry_get;
    registry_get_plugin_list_t get_plugin_list;
    plugin_get_filename_t plugin_get_filename;
    plugin_load_t plugin_load;
    plugin_list_free_t plugin_list_free;
    const char *plugin_dir;
    GError *error = NULL;
    struct dirent *entry;
    GList *plugins, *l;
    DIR *dir;
    uint32_t n;
    void *gst;

    if (argc < 2)
    {
        printf("usage: %s <libgstreamer-1.0.dylib> <element>...\n", argv[0]);
        return FAIL_DLOPEN;
    }
    if (!(plugin_dir = getenv("GST_PLUGIN_SYSTEM_PATH_1_0")))
    {
        printf("gst: GST_PLUGIN_SYSTEM_PATH_1_0 is not set\n");
        return FAIL_DLOPEN;
    }
    if (!(gst = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("gst: cannot load %s: %s\n", argv[1], dlerror());
        return FAIL_DLOPEN;
    }
    init_check = (init_check_t)dlsym(gst, "gst_init_check");
    factory_make = (factory_make_t)dlsym(gst, "gst_element_factory_make");
    object_unref = (object_unref_t)dlsym(gst, "gst_object_unref");
    version_string = (version_string_t)dlsym(gst, "gst_version_string");
    registry_get = (registry_get_t)dlsym(gst, "gst_registry_get");
    get_plugin_list = (registry_get_plugin_list_t)dlsym(gst, "gst_registry_get_plugin_list");
    plugin_get_filename = (plugin_get_filename_t)dlsym(gst, "gst_plugin_get_filename");
    plugin_load = (plugin_load_t)dlsym(gst, "gst_plugin_load");
    plugin_list_free = (plugin_list_free_t)dlsym(gst, "gst_plugin_list_free");
    if (!init_check || !factory_make || !object_unref || !version_string || !registry_get ||
        !get_plugin_list || !plugin_get_filename || !plugin_load || !plugin_list_free)
    {
        printf("gst: %s lacks a GStreamer entry point\n", argv[1]);
        return FAIL_SYMBOLS;
    }

    if (!init_check(&init_argc, &init_args, &error))
    {
        printf("gst: gst_init_check failed: %s\n", error ? error->message : "no error message");
        return FAIL_INIT;
    }
    printf("gst: %s\n", version_string());

    /* Every plugin file must be in the registry and load. gst_plugin_load
     * returns a plugin that is loaded already, and loads one only cached in
     * the registry; a plugin whose initialization failed is in the registry
     * only as a blacklisted entry, and does not load. */
    if (!(dir = opendir(plugin_dir)))
    {
        printf("gst: cannot read %s\n", plugin_dir);
        return FAIL_PLUGIN;
    }
    plugins = get_plugin_list(registry_get());
    while ((entry = readdir(dir)))
    {
        size_t len = strlen(entry->d_name);
        void *loaded = NULL;

        if (len < 6 || strcmp(entry->d_name + len - 6, ".dylib")) continue;
        for (l = plugins; l && !loaded; l = l->next)
        {
            const char *file = plugin_get_filename(l->data);
            if (file && !strcmp(base_name(file), entry->d_name)) loaded = plugin_load(l->data);
        }
        if (!loaded)
        {
            printf("gst: the plugin %s is not in the registry or does not load\n", entry->d_name);
            failed_plugin = 1;
            continue;
        }
        printf("gst: registered %s\n", entry->d_name);
        object_unref(loaded);
    }
    closedir(dir);
    plugin_list_free(plugins);

    for (i = 2; i < argc; i++)
    {
        void *element = factory_make(argv[i], NULL);

        if (!element)
        {
            printf("gst: cannot create the element %s\n", argv[i]);
            failed_element = 1;
            continue;
        }
        printf("gst: created %s\n", argv[i]);
        object_unref(element);
    }

    for (n = 0; n < _dyld_image_count(); n++)
    {
        const char *name = _dyld_get_image_name(n), *base = base_name(name);
        size_t p;

        if (!strncmp(name, "/usr/lib/", 9) || !strncmp(name, "/System/", 8)) continue;
        for (p = 0; p < sizeof(prefixes) / sizeof(prefixes[0]); p++)
        {
            if (!strncmp(base, prefixes[p], strlen(prefixes[p])))
            {
                printf("gst: loaded %s\n", name);
                break;
            }
        }
    }
    if (failed_plugin) return FAIL_PLUGIN;
    return failed_element ? FAIL_ELEMENT : 0;
}
