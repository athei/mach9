/*
 * vkr64view <libvulkan.1.dylib>: make KosmicKrisp warn on the standard error
 * that 64-bit atomics on a view take its lock, as bundle-wine.sh checks.
 *
 * Writes a storage image descriptor of an R64_UINT view of a mutable
 * R32G32_UINT storage image, then two of an R64_UINT view of a mutable
 * R32G32_SINT storage image. Only the first image gets Metal's atomic usage
 * (int64_views in kk_image_layout_init needs the Metal format R32G32_UINT),
 * so 64-bit atomics on the second take KosmicKrisp's lock, which it has to
 * warn about once.
 *
 * Prints a line per step and exits with 0 when the descriptors were written
 * (the caller counts the driver's lines), 2 when a step failed, and 3 when
 * the device lacks R64_UINT storage images. Build it against the Vulkan
 * headers (-I) the loader was built with.
 */
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>

#include <dlfcn.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#define FUNCS(X)                                                                                   \
    X(vkEnumeratePhysicalDevices)                                                                  \
    X(vkGetPhysicalDeviceProperties)                                                               \
    X(vkGetPhysicalDeviceMemoryProperties)                                                         \
    X(vkGetPhysicalDeviceQueueFamilyProperties)                                                    \
    X(vkGetPhysicalDeviceFormatProperties)                                                         \
    X(vkCreateDevice)                                                                              \
    X(vkCreateImage)                                                                               \
    X(vkGetImageMemoryRequirements)                                                                \
    X(vkBindImageMemory)                                                                           \
    X(vkCreateImageView)                                                                           \
    X(vkAllocateMemory)                                                                            \
    X(vkCreateDescriptorSetLayout)                                                                 \
    X(vkCreateDescriptorPool)                                                                      \
    X(vkAllocateDescriptorSets)                                                                    \
    X(vkUpdateDescriptorSets)

#define DECLARE(name) static PFN_##name name;
FUNCS(DECLARE)
#undef DECLARE

static VkInstance instance;
static VkPhysicalDevice physical;
static VkPhysicalDeviceMemoryProperties memory_properties;
static VkDevice device;
static VkDescriptorSetLayout set_layout;
static VkDescriptorPool descriptor_pool;

static bool ok(VkResult result, const char *what)
{
    if (result == VK_SUCCESS) return true;
    printf("vkr64view: %s failed: %d\n", what, (int)result);
    return false;
}

static bool allocate(VkMemoryRequirements req, VkMemoryPropertyFlags flags, VkDeviceMemory *out)
{
    VkMemoryAllocateInfo info = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                                  .allocationSize = req.size };
    uint32_t i;

    for (i = 0; i < memory_properties.memoryTypeCount; i++)
        if ((req.memoryTypeBits & (1u << i)) &&
            (memory_properties.memoryTypes[i].propertyFlags & flags) == flags)
            break;
    if (i == memory_properties.memoryTypeCount)
    {
        printf("vkr64view: no memory type with flags %#x in %#x\n", flags, req.memoryTypeBits);
        return false;
    }
    info.memoryTypeIndex = i;
    return ok(vkAllocateMemory(device, &info, NULL, out), "vkAllocateMemory");
}

static bool create_device(void)
{
    float priority = 1.0f;
    VkDeviceQueueCreateInfo queue_info = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                           .queueCount = 1,
                                           .pQueuePriorities = &priority };
    VkDeviceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                                .queueCreateInfoCount = 1,
                                .pQueueCreateInfos = &queue_info };
    VkQueueFamilyProperties families[16];
    VkPhysicalDeviceProperties props;
    uint32_t count = 1, i;
    VkResult res;

    res = vkEnumeratePhysicalDevices(instance, &count, &physical);
    if ((res != VK_SUCCESS && res != VK_INCOMPLETE) || !count)
    {
        printf("vkr64view: no Vulkan device, result %d\n", (int)res);
        return false;
    }
    vkGetPhysicalDeviceProperties(physical, &props);
    vkGetPhysicalDeviceMemoryProperties(physical, &memory_properties);
    printf("vkr64view: device %s, driverVersion %#x\n", props.deviceName, props.driverVersion);
    count = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(physical, &count, families);
    for (i = 0; i < count; i++)
        if (families[i].queueFlags & VK_QUEUE_COMPUTE_BIT) break;
    if (i == count)
    {
        printf("vkr64view: no queue family with compute\n");
        return false;
    }
    queue_info.queueFamilyIndex = i;
    return ok(vkCreateDevice(physical, &info, NULL, &device), "vkCreateDevice");
}

/* A descriptor set layout of one storage image binding and a pool for
 * <sets> sets of it. */
static bool create_set_layout(uint32_t sets)
{
    VkDescriptorSetLayoutBinding binding = { 0, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1,
                                             VK_SHADER_STAGE_COMPUTE_BIT, NULL };
    VkDescriptorSetLayoutCreateInfo set_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        .bindingCount = 1,
        .pBindings = &binding
    };
    VkDescriptorPoolSize size = { VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, sets };
    VkDescriptorPoolCreateInfo pool = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                                        .maxSets = sets,
                                        .poolSizeCount = 1,
                                        .pPoolSizes = &size };

    return ok(vkCreateDescriptorSetLayout(device, &set_info, NULL, &set_layout),
              "vkCreateDescriptorSetLayout") &&
           ok(vkCreateDescriptorPool(device, &pool, NULL, &descriptor_pool),
              "vkCreateDescriptorPool");
}

static bool allocate_set(VkDescriptorSet *set)
{
    VkDescriptorSetAllocateInfo info = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                                         .descriptorPool = descriptor_pool,
                                         .descriptorSetCount = 1,
                                         .pSetLayouts = &set_layout };

    return ok(vkAllocateDescriptorSets(device, &info, set), "vkAllocateDescriptorSets");
}

/* Writes <writes> storage image descriptors of an R64_UINT view of a
 * mutable 4x4 storage image of <format>. */
static bool bind_r64_view(VkFormat format, const char *name, int writes)
{
    VkFormat formats[2] = { format, VK_FORMAT_R64_UINT };
    VkImageFormatListCreateInfo list = { .sType = VK_STRUCTURE_TYPE_IMAGE_FORMAT_LIST_CREATE_INFO,
                                         .viewFormatCount = 2,
                                         .pViewFormats = formats };
    VkImageCreateInfo image_info = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                                     .pNext = &list,
                                     .flags = VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT,
                                     .imageType = VK_IMAGE_TYPE_2D,
                                     .format = format,
                                     .extent = { 4, 4, 1 },
                                     .mipLevels = 1,
                                     .arrayLayers = 1,
                                     .samples = VK_SAMPLE_COUNT_1_BIT,
                                     .tiling = VK_IMAGE_TILING_OPTIMAL,
                                     .usage = VK_IMAGE_USAGE_STORAGE_BIT,
                                     .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
                                     .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    VkImageViewCreateInfo view_info = { .sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                                        .viewType = VK_IMAGE_VIEW_TYPE_2D,
                                        .format = VK_FORMAT_R64_UINT,
                                        .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0,
                                                              1 } };
    VkDescriptorImageInfo image_desc = { .imageLayout = VK_IMAGE_LAYOUT_GENERAL };
    VkWriteDescriptorSet write = { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                                   .descriptorCount = 1,
                                   .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
                                   .pImageInfo = &image_desc };
    VkMemoryRequirements req;
    VkDeviceMemory memory;
    VkImage image;
    int i;

    if (!ok(vkCreateImage(device, &image_info, NULL, &image), "vkCreateImage")) return false;
    vkGetImageMemoryRequirements(device, image, &req);
    if (!allocate(req, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, &memory) ||
        !ok(vkBindImageMemory(device, image, memory, 0), "vkBindImageMemory"))
        return false;
    view_info.image = image;
    if (!ok(vkCreateImageView(device, &view_info, NULL, &image_desc.imageView),
            "vkCreateImageView"))
        return false;
    for (i = 0; i < writes; i++)
    {
        if (!allocate_set(&write.dstSet)) return false;
        vkUpdateDescriptorSets(device, 1, &write, 0, NULL);
        printf("vkr64view: bound an R64_UINT view of an %s image\n", name);
    }
    return true;
}

int main(int argc, char **argv)
{
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .pApplicationName = "mach9 vkr64view",
                              .apiVersion = VK_API_VERSION_1_3 };
    VkInstanceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                  .pApplicationInfo = &app };
    PFN_vkGetInstanceProcAddr get_instance_proc;
    PFN_vkCreateInstance create_instance;
    VkFormatProperties props;
    void *loader;

    setvbuf(stdout, NULL, _IOLBF, 0);
    if (argc != 2)
    {
        printf("usage: %s <libvulkan.1.dylib>\n", argv[0]);
        return 2;
    }
    if (!(loader = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("vkr64view: cannot load %s: %s\n", argv[1], dlerror());
        return 2;
    }
    get_instance_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    if (!get_instance_proc)
    {
        printf("vkr64view: %s has no vkGetInstanceProcAddr\n", argv[1]);
        return 2;
    }
    create_instance = (PFN_vkCreateInstance)get_instance_proc(NULL, "vkCreateInstance");
    if (!create_instance || !ok(create_instance(&info, NULL, &instance), "vkCreateInstance"))
        return 2;
#define LOAD(name)                                                                                 \
    if (!(name = (PFN_##name)get_instance_proc(instance, #name)))                                  \
    {                                                                                              \
        printf("vkr64view: the loader has no %s\n", #name);                                        \
        return 2;                                                                                  \
    }
    FUNCS(LOAD)
#undef LOAD
    if (!create_device()) return 2;

    vkGetPhysicalDeviceFormatProperties(physical, VK_FORMAT_R64_UINT, &props);
    if (!(props.optimalTilingFeatures & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT))
    {
        printf("vkr64view: the device has no R64_UINT storage images\n");
        return 3;
    }
    /* The process exits without destroying anything: the check only needs
     * the driver's lines. */
    if (!create_set_layout(3) || !bind_r64_view(VK_FORMAT_R32G32_UINT, "R32G32_UINT", 1) ||
        !bind_r64_view(VK_FORMAT_R32G32_SINT, "R32G32_SINT", 2))
        return 2;
    return 0;
}
