/* The Vulkan program bundle-wine.sh builds for each x86 PE architecture of a
 * bundle with Vulkan and runs under the bundled wine. It loads vulkan-1.dll,
 * Wine's loader, creates an instance without any extension, so no window is
 * needed, enumerates the physical devices and prints the name of each. That
 * goes through winevulkan and win32u to the host loader, which finds the
 * bundled driver through its manifest. Each device must also offer
 * VK_EXT_transform_feedback, which upstream KosmicKrisp does not implement:
 * a commit of our Mesa fork adds it, advertised whatever
 * MESA_KK_EXPERIMENTAL says (older builds of the fork advertised it only
 * when that variable named it), so a missing extension means the driver is
 * not the current fork's. It prints one line per step that fails.
 *
 * The few Vulkan declarations it needs are written out here, since
 * llvm-mingw has no Vulkan headers. Their layout is that of vulkan_core.h;
 * VkPhysicalDeviceProperties is only read up to deviceName, and the buffer
 * the driver fills is larger than the whole structure. */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>

#if defined(__x86_64__)
#define ARCH "x86_64"
#elif defined(__i386__)
#define ARCH "i686"
#elif defined(__aarch64__)
#define ARCH "aarch64"
#else
#error "unknown architecture"
#endif

/* VKAPI_PTR is __stdcall on 32-bit Windows, and nothing on 64-bit. */
#define VKAPI WINAPI

typedef struct VkInstance_T *VkInstance;
typedef struct VkPhysicalDevice_T *VkPhysicalDevice;
typedef int32_t VkResult;

typedef struct
{
    int32_t sType;
    const void *pNext;
    const char *pApplicationName;
    uint32_t applicationVersion;
    const char *pEngineName;
    uint32_t engineVersion;
    uint32_t apiVersion;
} VkApplicationInfo;

typedef struct
{
    int32_t sType;
    const void *pNext;
    uint32_t flags;
    const VkApplicationInfo *pApplicationInfo;
    uint32_t enabledLayerCount;
    const char *const *ppEnabledLayerNames;
    uint32_t enabledExtensionCount;
    const char *const *ppEnabledExtensionNames;
} VkInstanceCreateInfo;

typedef struct
{
    uint32_t apiVersion;
    uint32_t driverVersion;
    uint32_t vendorID;
    uint32_t deviceID;
    int32_t deviceType;
    char deviceName[256];
} VkPhysicalDevicePropertiesHead;

typedef struct
{
    char extensionName[256];
    uint32_t specVersion;
} VkExtensionProperties;

#define VK_STRUCTURE_TYPE_APPLICATION_INFO 0
#define VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO 1
#define VK_INCOMPLETE 5
#define VK_API_VERSION_1_1 ((1u << 22) | (1u << 12))

typedef void (VKAPI *PFN_vkVoidFunction)(void);
typedef PFN_vkVoidFunction (VKAPI *PFN_vkGetInstanceProcAddr)(VkInstance, const char *);
typedef VkResult (VKAPI *PFN_vkCreateInstance)(const VkInstanceCreateInfo *, const void *,
                                               VkInstance *);
typedef VkResult (VKAPI *PFN_vkEnumeratePhysicalDevices)(VkInstance, uint32_t *,
                                                         VkPhysicalDevice *);
typedef void (VKAPI *PFN_vkGetPhysicalDeviceProperties)(VkPhysicalDevice, void *);
typedef VkResult (VKAPI *PFN_vkEnumerateDeviceExtensionProperties)(VkPhysicalDevice,
                                                                   const char *, uint32_t *,
                                                                   VkExtensionProperties *);
typedef void (VKAPI *PFN_vkDestroyInstance)(VkInstance, const void *);

static PFN_vkGetInstanceProcAddr get_proc;

/* The function <name> of <instance>, or NULL after a line saying it is
 * missing. */
static PFN_vkVoidFunction load(VkInstance instance, const char *name)
{
    PFN_vkVoidFunction func = get_proc(instance, name);

    if (!func) printf("wine-build smoke test: %s vkGetInstanceProcAddr has no %s\n", ARCH, name);
    return func;
}

/* Whether <device> offers the device extension <name>. */
static int has_extension(PFN_vkEnumerateDeviceExtensionProperties enumerate,
                         VkPhysicalDevice device, const char *name)
{
    VkExtensionProperties *props;
    uint32_t count = 0, i;
    int found = 0;

    if (enumerate(device, NULL, &count, NULL) || !count) return 0;
    if (!(props = calloc(count, sizeof(*props)))) return 0;
    if (!enumerate(device, NULL, &count, props))
        for (i = 0; i < count && !found; i++) found = !strcmp(props[i].extensionName, name);
    free(props);
    return found;
}

int main(void)
{
    static union { VkPhysicalDevicePropertiesHead head; char bytes[4096]; } props;
    static const char xfb[] = "VK_EXT_transform_feedback";
    VkApplicationInfo app = { VK_STRUCTURE_TYPE_APPLICATION_INFO, NULL, "wine-build smoke test",
                              1, NULL, 0, VK_API_VERSION_1_1 };
    VkInstanceCreateInfo info = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, NULL, 0, &app,
                                  0, NULL, 0, NULL };
    PFN_vkCreateInstance create_instance;
    PFN_vkEnumeratePhysicalDevices enumerate;
    PFN_vkGetPhysicalDeviceProperties get_properties;
    PFN_vkEnumerateDeviceExtensionProperties enumerate_extensions;
    PFN_vkDestroyInstance destroy_instance;
    VkPhysicalDevice devices[8];
    uint32_t count = 8, i;
    VkInstance instance;
    HMODULE vulkan;
    VkResult res;
    int failed = 0;

    if (!(vulkan = LoadLibraryA("vulkan-1.dll")))
    {
        printf("wine-build smoke test: %s cannot load vulkan-1.dll, error %lu\n", ARCH,
               GetLastError());
        return 1;
    }
    get_proc = (PFN_vkGetInstanceProcAddr)(void *)GetProcAddress(vulkan, "vkGetInstanceProcAddr");
    if (!get_proc)
    {
        printf("wine-build smoke test: %s vulkan-1.dll has no vkGetInstanceProcAddr\n", ARCH);
        return 1;
    }
    if (!(create_instance = (PFN_vkCreateInstance)load(NULL, "vkCreateInstance"))) return 1;
    if ((res = create_instance(&info, NULL, &instance)))
    {
        printf("wine-build smoke test: %s vkCreateInstance failed, result %d\n", ARCH, (int)res);
        return 1;
    }
    enumerate = (PFN_vkEnumeratePhysicalDevices)load(instance, "vkEnumeratePhysicalDevices");
    get_properties = (PFN_vkGetPhysicalDeviceProperties)load(instance,
                                                             "vkGetPhysicalDeviceProperties");
    enumerate_extensions = (PFN_vkEnumerateDeviceExtensionProperties)load(
        instance, "vkEnumerateDeviceExtensionProperties");
    destroy_instance = (PFN_vkDestroyInstance)load(instance, "vkDestroyInstance");
    if (!enumerate || !get_properties || !enumerate_extensions || !destroy_instance) return 1;
    res = enumerate(instance, &count, devices);
    if ((res && res != VK_INCOMPLETE) || !count)
    {
        printf("wine-build smoke test: %s finds no Vulkan device, result %d\n", ARCH, (int)res);
        return 1;
    }
    for (i = 0; i < count; i++)
    {
        get_properties(devices[i], &props);
        printf("wine-build smoke test: %s vulkan device %.256s\n", ARCH, props.head.deviceName);
        if (!has_extension(enumerate_extensions, devices[i], xfb))
        {
            printf("wine-build smoke test: %s device %.256s does not offer %s, which "
                   "the Mesa fork's KosmicKrisp always advertises\n",
                   ARCH, props.head.deviceName, xfb);
            failed = 1;
        }
    }
    destroy_instance(instance, NULL);
    return failed;
}
