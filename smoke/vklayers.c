/* The host program bundle-wine.sh builds to list the instance layers the
 * bundle's Vulkan loader finds, the first argument, with the environment it
 * is run with (VK_LAYER_PATH naming share/vulkan/explicit_layer.d). It
 * needs no driver and no GPU: the loader reads the layer manifests without
 * one. It prints each layer's name and exits with 0 if the second argument
 * is among them, 1 if it is not, and 2 if the loader does not load or does
 * not list its layers. The few Vulkan types it uses are declared here, so
 * it builds without the Vulkan headers. */

#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct
{
    char layerName[256];
    uint32_t specVersion;
    uint32_t implementationVersion;
    char description[256];
} VkLayerProperties;

typedef void (*PFN_vkVoidFunction)(void);
typedef PFN_vkVoidFunction (*PFN_vkGetInstanceProcAddr)(void *instance, const char *name);
typedef int32_t (*PFN_vkEnumerateInstanceLayerProperties)(uint32_t *count, VkLayerProperties *properties);

int main(int argc, char **argv)
{
    PFN_vkGetInstanceProcAddr get_proc;
    PFN_vkEnumerateInstanceLayerProperties enumerate;
    VkLayerProperties *layers;
    uint32_t i, count = 0;
    void *loader;
    int found = 0;

    if (argc != 3)
    {
        fprintf(stderr, "usage: %s <libvulkan.1.dylib> <layer name>\n", argv[0]);
        return 2;
    }
    if (!(loader = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        fprintf(stderr, "vklayers: %s\n", dlerror());
        return 2;
    }
    get_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    enumerate = get_proc ? (PFN_vkEnumerateInstanceLayerProperties)get_proc(NULL, "vkEnumerateInstanceLayerProperties") : NULL;
    if (!enumerate || enumerate(&count, NULL) || !(layers = calloc(count + 1, sizeof(*layers))) ||
        enumerate(&count, layers))
    {
        fprintf(stderr, "vklayers: the loader does not list its instance layers\n");
        return 2;
    }
    for (i = 0; i < count; i++)
    {
        printf("vklayers: %s\n", layers[i].layerName);
        if (!strcmp(layers[i].layerName, argv[2])) found = 1;
    }
    return found ? 0 : 1;
}
