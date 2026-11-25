/*
 * vkatomic64 <libvulkan.1.dylib> [<adds>]: check that 64-bit image atomics on one
 * texel are atomic when they reach it through different views. KosmicKrisp
 * runs imageAtomicAdd on R64_UINT under a lock it picks from the texel's
 * coordinates; a driver that takes the coordinates of the view, which start
 * at the view's base array layer, gives the two views of one texel two locks,
 * and adds through one view overwrite adds through the other.
 *
 * Every invocation of a dispatch of <adds> invocations, ADDS by default and a
 * multiple of GROUP_SIZE, adds 1 to the same texel (vkatomic64.comp). The
 * cases:
 *
 *   one-view    texel (0, 0) of layer 1 of a two-layer 2D array image, all
 *               adds through the 2D array view; the control, one lock on
 *               any driver
 *   two-views   the same texel, half the adds through a 2D view of layer 1
 *               and half through the 2D array view
 *   3d-slice    texel (0, 0) of slice 1 of a 3D image of depth 2, half the
 *               adds through a 2D view of slice 1
 *               (VK_EXT_image_2d_view_of_3d) and half through the 3D view
 *
 * Each case runs ROUNDS dispatches, each on a texel set to 0 by a copy before
 * it, and reads the texel back after it. Prints a line per case with the
 * rounds whose sum was not <adds>. Exits with 2 as soon as a case cannot run, a
 * missing feature, a failed setup, submit or wait included, else with 1 when
 * a round lost an add, 0 otherwise. A watchdog ends a run that hangs with 2
 * after a few seconds. Build it against the Vulkan headers (-I) the loader
 * was built with.
 */
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>

#include <dlfcn.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "vkatomic64.comp.h"

#define GROUP_SIZE 64 /* local_size_x of vkatomic64.comp */
#define ADDS (1u << 12)
#define ROUNDS 10
#define WATCHDOG_SECONDS 5
#define FENCE_TIMEOUT_NS 3000000000ull

#define FUNCS(X)                                                                                   \
    X(vkEnumeratePhysicalDevices)                                                                  \
    X(vkGetPhysicalDeviceProperties)                                                               \
    X(vkGetPhysicalDeviceFeatures2)                                                                \
    X(vkGetPhysicalDeviceFormatProperties)                                                         \
    X(vkGetPhysicalDeviceMemoryProperties)                                                         \
    X(vkGetPhysicalDeviceQueueFamilyProperties)                                                    \
    X(vkCreateDevice)                                                                              \
    X(vkGetDeviceQueue)                                                                            \
    X(vkCreateBuffer)                                                                              \
    X(vkGetBufferMemoryRequirements)                                                               \
    X(vkBindBufferMemory)                                                                          \
    X(vkCreateImage)                                                                               \
    X(vkGetImageMemoryRequirements)                                                                \
    X(vkBindImageMemory)                                                                           \
    X(vkCreateImageView)                                                                           \
    X(vkAllocateMemory)                                                                            \
    X(vkMapMemory)                                                                                 \
    X(vkCreateShaderModule)                                                                        \
    X(vkDestroyShaderModule)                                                                       \
    X(vkCreateDescriptorSetLayout)                                                                 \
    X(vkCreatePipelineLayout)                                                                      \
    X(vkCreateComputePipelines)                                                                    \
    X(vkCreateDescriptorPool)                                                                      \
    X(vkAllocateDescriptorSets)                                                                    \
    X(vkUpdateDescriptorSets)                                                                      \
    X(vkCreateCommandPool)                                                                         \
    X(vkAllocateCommandBuffers)                                                                    \
    X(vkResetCommandBuffer)                                                                        \
    X(vkBeginCommandBuffer)                                                                        \
    X(vkEndCommandBuffer)                                                                          \
    X(vkCreateFence)                                                                               \
    X(vkResetFences)                                                                               \
    X(vkWaitForFences)                                                                             \
    X(vkQueueSubmit)                                                                               \
    X(vkCmdBindPipeline)                                                                           \
    X(vkCmdBindDescriptorSets)                                                                     \
    X(vkCmdPushConstants)                                                                          \
    X(vkCmdDispatch)                                                                               \
    X(vkCmdPipelineBarrier2)                                                                       \
    X(vkCmdCopyBufferToImage)                                                                      \
    X(vkCmdCopyImageToBuffer)

#define DECLARE(name) static PFN_##name name;
FUNCS(DECLARE)
#undef DECLARE

enum mode
{
    MODE_ONE_VIEW,
    MODE_TWO_VIEWS,
    MODE_3D_SLICE,
};

struct atomic_case
{
    const char *name;
    enum mode mode;
};

static const struct atomic_case cases[] = {
    { "one-view", MODE_ONE_VIEW },
    { "two-views", MODE_TWO_VIEWS },
    { "3d-slice", MODE_3D_SLICE },
};

struct buffer
{
    VkBuffer handle;
    void *map;
};

static VkInstance instance;
static VkPhysicalDevice physical;
static VkPhysicalDeviceMemoryProperties memory_properties;
static VkDevice device;
static VkQueue queue;
static uint32_t queue_family;
static VkCommandBuffer cmd;
static VkFence fence;
static VkPipelineLayout pipeline_layout;
static VkPipeline pipeline;
/* The 2D array image of one-view and two-views, and the 3D image of
 * 3d-slice. */
static VkImage layered, volume;
/* The 2D view of layer 1 and the 2D array view of both layers of layered;
 * the 3D view and the 2D view of slice 1 of volume. */
static VkImageView layer_view, array_view, volume_view, slice_view;
/* The set of one-view and two-views binds layer_view as the shader's single
 * image, the set of 3d-slice binds slice_view. */
static VkDescriptorSet layered_set, volume_set;
static struct buffer zero, readback;
static uint32_t adds = ADDS;

static bool ok(VkResult result, const char *what)
{
    if (result == VK_SUCCESS) return true;
    printf("vkatomic64: %s failed: %d\n", what, (int)result);
    return false;
}

static void watchdog(int sig)
{
    static const char message[] = "vkatomic64: no result within the watchdog's limit\n";

    (void)sig;
    (void)write(STDOUT_FILENO, message, sizeof(message) - 1);
    _exit(2);
}

static void start_watchdog(void)
{
    struct sigaction action = { .sa_handler = watchdog };

    sigemptyset(&action.sa_mask);
    sigaction(SIGALRM, &action, NULL);
    alarm(WATCHDOG_SECONDS);
}

static int find_memory_type(uint32_t bits, VkMemoryPropertyFlags flags)
{
    uint32_t i;

    for (i = 0; i < memory_properties.memoryTypeCount; i++)
        if ((bits & (1u << i)) &&
            (memory_properties.memoryTypes[i].propertyFlags & flags) == flags)
            return (int)i;
    return -1;
}

static bool allocate(VkMemoryRequirements req, VkMemoryPropertyFlags flags, VkDeviceMemory *out)
{
    int type = find_memory_type(req.memoryTypeBits, flags);
    VkMemoryAllocateInfo info = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                                  .allocationSize = req.size };

    if (type < 0)
    {
        printf("vkatomic64: no memory type with flags %#x in %#x\n", flags, req.memoryTypeBits);
        return false;
    }
    info.memoryTypeIndex = (uint32_t)type;
    return ok(vkAllocateMemory(device, &info, NULL, out), "vkAllocateMemory");
}

/* A host-visible, mapped, zeroed buffer with memory of its own. */
static bool create_buffer(VkDeviceSize size, VkBufferUsageFlags usage, struct buffer *out)
{
    VkBufferCreateInfo info = { .sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
                                .size = size,
                                .usage = usage,
                                .sharingMode = VK_SHARING_MODE_EXCLUSIVE };
    VkMemoryRequirements req;
    VkDeviceMemory memory;

    if (!ok(vkCreateBuffer(device, &info, NULL, &out->handle), "vkCreateBuffer")) return false;
    vkGetBufferMemoryRequirements(device, out->handle, &req);
    if (!allocate(req, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                  &memory) ||
        !ok(vkBindBufferMemory(device, out->handle, memory, 0), "vkBindBufferMemory") ||
        !ok(vkMapMemory(device, memory, 0, VK_WHOLE_SIZE, 0, &out->map), "vkMapMemory"))
        return false;
    memset(out->map, 0, size);
    return true;
}

/* A 4x4 R64_UINT storage image with two layers (2D) or two slices (3D). */
static bool create_image(VkImageType type, VkImage *out)
{
    VkImageCreateInfo info = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                               .imageType = type,
                               .format = VK_FORMAT_R64_UINT,
                               .extent = { 4, 4, type == VK_IMAGE_TYPE_3D ? 2 : 1 },
                               .mipLevels = 1,
                               .arrayLayers = type == VK_IMAGE_TYPE_3D ? 1 : 2,
                               .samples = VK_SAMPLE_COUNT_1_BIT,
                               .tiling = VK_IMAGE_TILING_OPTIMAL,
                               .usage = VK_IMAGE_USAGE_STORAGE_BIT |
                                        VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                                        VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                               .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
                               .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    VkMemoryRequirements req;
    VkDeviceMemory memory;

    if (type == VK_IMAGE_TYPE_3D) info.flags = VK_IMAGE_CREATE_2D_VIEW_COMPATIBLE_BIT_EXT;
    if (!ok(vkCreateImage(device, &info, NULL, out), "vkCreateImage")) return false;
    vkGetImageMemoryRequirements(device, *out, &req);
    return allocate(req, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, &memory) &&
           ok(vkBindImageMemory(device, *out, memory, 0), "vkBindImageMemory");
}

static bool create_view(VkImage image, VkImageViewType type, uint32_t base_layer,
                        uint32_t layers, VkImageView *out)
{
    VkImageViewCreateInfo info = { .sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                                   .image = image,
                                   .viewType = type,
                                   .format = VK_FORMAT_R64_UINT,
                                   .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1,
                                                         base_layer, layers } };

    return ok(vkCreateImageView(device, &info, NULL, out), "vkCreateImageView");
}

static bool create_device(void)
{
    static const char *const extensions[] = { VK_EXT_SHADER_IMAGE_ATOMIC_INT64_EXTENSION_NAME,
                                              VK_EXT_IMAGE_2D_VIEW_OF_3D_EXTENSION_NAME };
    VkPhysicalDeviceImage2DViewOf3DFeaturesEXT view_of_3d = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_2D_VIEW_OF_3D_FEATURES_EXT
    };
    VkPhysicalDeviceShaderImageAtomicInt64FeaturesEXT image_int64 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_IMAGE_ATOMIC_INT64_FEATURES_EXT,
        .pNext = &view_of_3d
    };
    VkPhysicalDeviceVulkan13Features features13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        .pNext = &image_int64
    };
    VkPhysicalDeviceFeatures2 features = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                                           .pNext = &features13 };
    VkPhysicalDeviceImage2DViewOf3DFeaturesEXT enable_view_of_3d = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_2D_VIEW_OF_3D_FEATURES_EXT,
        .image2DViewOf3D = VK_TRUE
    };
    VkPhysicalDeviceShaderImageAtomicInt64FeaturesEXT enable_image_int64 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_IMAGE_ATOMIC_INT64_FEATURES_EXT,
        .pNext = &enable_view_of_3d,
        .shaderImageInt64Atomics = VK_TRUE
    };
    VkPhysicalDeviceVulkan13Features enable13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        .pNext = &enable_image_int64,
        .synchronization2 = VK_TRUE
    };
    VkPhysicalDeviceFeatures2 enable = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                                         .pNext = &enable13,
                                         .features.shaderInt64 = VK_TRUE };
    float priority = 1.0f;
    VkDeviceQueueCreateInfo queue_info = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                           .queueCount = 1,
                                           .pQueuePriorities = &priority };
    VkDeviceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                                .pNext = &enable,
                                .queueCreateInfoCount = 1,
                                .pQueueCreateInfos = &queue_info,
                                .enabledExtensionCount = 2,
                                .ppEnabledExtensionNames = extensions };
    VkQueueFamilyProperties families[16];
    VkPhysicalDeviceProperties props;
    VkFormatProperties format;
    uint32_t count = 1, i;
    VkResult res;

    res = vkEnumeratePhysicalDevices(instance, &count, &physical);
    if ((res != VK_SUCCESS && res != VK_INCOMPLETE) || !count)
    {
        printf("vkatomic64: no Vulkan device, result %d\n", (int)res);
        return false;
    }
    vkGetPhysicalDeviceProperties(physical, &props);
    vkGetPhysicalDeviceMemoryProperties(physical, &memory_properties);
    vkGetPhysicalDeviceFeatures2(physical, &features);
    vkGetPhysicalDeviceFormatProperties(physical, VK_FORMAT_R64_UINT, &format);
    printf("vkatomic64: device %s, driverVersion %#x\n", props.deviceName, props.driverVersion);
    if (!features.features.shaderInt64 || !features13.synchronization2 ||
        !image_int64.shaderImageInt64Atomics || !view_of_3d.image2DViewOf3D)
    {
        printf("vkatomic64: the device lacks shaderInt64, synchronization2, "
               "shaderImageInt64Atomics or image2DViewOf3D\n");
        return false;
    }
    if (!(format.optimalTilingFeatures & VK_FORMAT_FEATURE_STORAGE_IMAGE_ATOMIC_BIT))
    {
        printf("vkatomic64: R64_UINT has no storage image atomics\n");
        return false;
    }
    count = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(physical, &count, families);
    for (i = 0; i < count; i++)
        if (families[i].queueFlags & VK_QUEUE_COMPUTE_BIT) break;
    if (i == count)
    {
        printf("vkatomic64: no queue family with compute\n");
        return false;
    }
    queue_family = queue_info.queueFamilyIndex = i;
    if (!ok(vkCreateDevice(physical, &info, NULL, &device), "vkCreateDevice")) return false;
    vkGetDeviceQueue(device, queue_family, 0, &queue);
    return true;
}

static void write_set(VkDescriptorSet set, VkImageView single)
{
    VkDescriptorImageInfo images[3] = {
        { VK_NULL_HANDLE, single, VK_IMAGE_LAYOUT_GENERAL },
        { VK_NULL_HANDLE, array_view, VK_IMAGE_LAYOUT_GENERAL },
        { VK_NULL_HANDLE, volume_view, VK_IMAGE_LAYOUT_GENERAL },
    };
    VkWriteDescriptorSet write = { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                                   .dstSet = set,
                                   .dstBinding = 0,
                                   .descriptorCount = 3,
                                   .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
                                   .pImageInfo = images };

    vkUpdateDescriptorSets(device, 1, &write, 0, NULL);
}

static bool create_pipeline(void)
{
    VkDescriptorSetLayoutBinding binding = { 0, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 3,
                                             VK_SHADER_STAGE_COMPUTE_BIT, NULL };
    VkDescriptorSetLayoutBinding bindings[3];
    VkDescriptorSetLayoutCreateInfo set_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        .bindingCount = 3,
        .pBindings = bindings
    };
    VkPushConstantRange push = { VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof(uint32_t) };
    VkPipelineLayoutCreateInfo layout_info = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                                               .setLayoutCount = 1,
                                               .pushConstantRangeCount = 1,
                                               .pPushConstantRanges = &push };
    VkDescriptorPoolSize size = { VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 6 };
    VkDescriptorPoolCreateInfo pool_info = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                                             .maxSets = 2,
                                             .poolSizeCount = 1,
                                             .pPoolSizes = &size };
    VkDescriptorSetLayout set_layouts[2];
    VkDescriptorSetAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                                          .descriptorSetCount = 2,
                                          .pSetLayouts = set_layouts };
    VkShaderModuleCreateInfo module_info = { .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
                                             .codeSize = sizeof(vkatomic64_comp),
                                             .pCode = vkatomic64_comp };
    VkComputePipelineCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        .stage = { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                   .stage = VK_SHADER_STAGE_COMPUTE_BIT,
                   .pName = "main" },
    };
    VkDescriptorSetLayout set_layout;
    VkDescriptorSet sets[2];
    VkDescriptorPool pool;
    bool created;
    uint32_t i;

    for (i = 0; i < 3; i++)
    {
        bindings[i] = binding;
        bindings[i].binding = i;
        bindings[i].descriptorCount = 1;
    }
    if (!ok(vkCreateDescriptorSetLayout(device, &set_info, NULL, &set_layout),
            "vkCreateDescriptorSetLayout"))
        return false;
    layout_info.pSetLayouts = &set_layout;
    set_layouts[0] = set_layouts[1] = set_layout;
    if (!ok(vkCreatePipelineLayout(device, &layout_info, NULL, &pipeline_layout),
            "vkCreatePipelineLayout") ||
        !ok(vkCreateDescriptorPool(device, &pool_info, NULL, &pool), "vkCreateDescriptorPool"))
        return false;
    alloc.descriptorPool = pool;
    if (!ok(vkAllocateDescriptorSets(device, &alloc, sets), "vkAllocateDescriptorSets"))
        return false;
    layered_set = sets[0];
    volume_set = sets[1];
    write_set(layered_set, layer_view);
    write_set(volume_set, slice_view);

    if (!ok(vkCreateShaderModule(device, &module_info, NULL, &info.stage.module),
            "vkCreateShaderModule"))
        return false;
    info.layout = pipeline_layout;
    created = ok(vkCreateComputePipelines(device, VK_NULL_HANDLE, 1, &info, NULL, &pipeline),
                 "vkCreateComputePipelines");
    vkDestroyShaderModule(device, info.stage.module, NULL);
    return created;
}

static bool create_objects(void)
{
    VkCommandPoolCreateInfo pool_info = { .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
                                          .flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
                                          .queueFamilyIndex = queue_family };
    VkCommandBufferAllocateInfo cmd_info = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                                             .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
                                             .commandBufferCount = 1 };
    VkFenceCreateInfo fence_info = { .sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
    VkCommandPool command_pool;

    if (!ok(vkCreateCommandPool(device, &pool_info, NULL, &command_pool), "vkCreateCommandPool"))
        return false;
    cmd_info.commandPool = command_pool;
    return ok(vkAllocateCommandBuffers(device, &cmd_info, &cmd), "vkAllocateCommandBuffers") &&
           ok(vkCreateFence(device, &fence_info, NULL, &fence), "vkCreateFence") &&
           create_buffer(sizeof(uint64_t), VK_BUFFER_USAGE_TRANSFER_SRC_BIT, &zero) &&
           create_buffer(sizeof(uint64_t), VK_BUFFER_USAGE_TRANSFER_DST_BIT, &readback) &&
           create_image(VK_IMAGE_TYPE_2D, &layered) && create_image(VK_IMAGE_TYPE_3D, &volume) &&
           create_view(layered, VK_IMAGE_VIEW_TYPE_2D, 1, 1, &layer_view) &&
           create_view(layered, VK_IMAGE_VIEW_TYPE_2D_ARRAY, 0, 2, &array_view) &&
           create_view(volume, VK_IMAGE_VIEW_TYPE_3D, 0, 1, &volume_view) &&
           create_view(volume, VK_IMAGE_VIEW_TYPE_2D, 1, 1, &slice_view) && create_pipeline();
}

static void barrier(VkPipelineStageFlags2 src_stage, VkAccessFlags2 src_access,
                    VkPipelineStageFlags2 dst_stage, VkAccessFlags2 dst_access)
{
    VkMemoryBarrier2 memory = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2,
                                .srcStageMask = src_stage,
                                .srcAccessMask = src_access,
                                .dstStageMask = dst_stage,
                                .dstAccessMask = dst_access };
    VkDependencyInfo dep = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                             .memoryBarrierCount = 1,
                             .pMemoryBarriers = &memory };

    vkCmdPipelineBarrier2(cmd, &dep);
}

/* Both images to VK_IMAGE_LAYOUT_GENERAL, once. */
static void to_general(void)
{
    VkImageMemoryBarrier2 images[2];
    VkDependencyInfo dep = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                             .imageMemoryBarrierCount = 2,
                             .pImageMemoryBarriers = images };
    uint32_t i;

    for (i = 0; i < 2; i++)
        images[i] = (VkImageMemoryBarrier2){
            .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
            .dstStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
            .dstAccessMask = VK_ACCESS_2_MEMORY_READ_BIT | VK_ACCESS_2_MEMORY_WRITE_BIT,
            .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
            .newLayout = VK_IMAGE_LAYOUT_GENERAL,
            .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .image = i ? volume : layered,
            .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, i ? 1 : 2 }
        };
    vkCmdPipelineBarrier2(cmd, &dep);
}

static bool submit(void)
{
    VkSubmitInfo info = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                          .commandBufferCount = 1,
                          .pCommandBuffers = &cmd };

    return ok(vkEndCommandBuffer(cmd), "vkEndCommandBuffer") &&
           ok(vkResetFences(device, 1, &fence), "vkResetFences") &&
           ok(vkQueueSubmit(queue, 1, &info, fence), "vkQueueSubmit") &&
           ok(vkWaitForFences(device, 1, &fence, VK_TRUE, FENCE_TIMEOUT_NS), "vkWaitForFences");
}

/* One round of <c>: the texel set to 0, <adds> adds, the texel read back
 * into <sum>. */
static bool run_round(const struct atomic_case *c, bool first, uint64_t *sum)
{
    VkCommandBufferBeginInfo begin = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                       .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VkBufferImageCopy region = { .imageExtent = { 1, 1, 1 } };
    uint32_t mode = c->mode;
    VkImage image;

    if (c->mode == MODE_3D_SLICE)
    {
        image = volume;
        region.imageSubresource = (VkImageSubresourceLayers){ VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1 };
        region.imageOffset.z = 1;
    }
    else
    {
        image = layered;
        region.imageSubresource = (VkImageSubresourceLayers){ VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 1 };
    }
    memset(readback.map, 0xff, sizeof(uint64_t));
    if (!ok(vkResetCommandBuffer(cmd, 0), "vkResetCommandBuffer") ||
        !ok(vkBeginCommandBuffer(cmd, &begin), "vkBeginCommandBuffer"))
        return false;
    if (first) to_general();
    barrier(VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT, VK_ACCESS_2_MEMORY_READ_BIT |
                                                      VK_ACCESS_2_MEMORY_WRITE_BIT,
            VK_PIPELINE_STAGE_2_COPY_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT);
    vkCmdCopyBufferToImage(cmd, zero.handle, image, VK_IMAGE_LAYOUT_GENERAL, 1, &region);
    barrier(VK_PIPELINE_STAGE_2_COPY_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
            VK_ACCESS_2_SHADER_STORAGE_READ_BIT | VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT);
    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
    vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline_layout, 0, 1,
                            c->mode == MODE_3D_SLICE ? &volume_set : &layered_set, 0, NULL);
    vkCmdPushConstants(cmd, pipeline_layout, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof(mode), &mode);
    vkCmdDispatch(cmd, adds / GROUP_SIZE, 1, 1);
    barrier(VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT,
            VK_PIPELINE_STAGE_2_COPY_BIT, VK_ACCESS_2_TRANSFER_READ_BIT);
    vkCmdCopyImageToBuffer(cmd, image, VK_IMAGE_LAYOUT_GENERAL, readback.handle, 1, &region);
    barrier(VK_PIPELINE_STAGE_2_COPY_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_2_HOST_BIT, VK_ACCESS_2_HOST_READ_BIT);
    if (!submit()) return false;
    memcpy(sum, readback.map, sizeof(*sum));
    return true;
}

/* Runs the rounds of one case; returns 0 when every round summed to <adds>,
 * 1 when one did not, 2 when a round could not run. */
static int run_case(const struct atomic_case *c, bool first)
{
    uint64_t sum, low = UINT64_MAX;
    unsigned lost = 0, round;
    struct timespec start, end;
    double ms;

    clock_gettime(CLOCK_MONOTONIC, &start);
    for (round = 0; round < ROUNDS; round++)
    {
        if (!run_round(c, first && !round, &sum)) return 2;
        if (sum != adds)
        {
            lost++;
            if (sum < low) low = sum;
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &end);
    ms = (double)(end.tv_sec - start.tv_sec) * 1e3 + (double)(end.tv_nsec - start.tv_nsec) / 1e6;
    if (lost)
    {
        printf("vkatomic64: %s: %u of %u rounds did not sum to %u, the lowest to %llu (%.0f ms)\n",
               c->name, lost, ROUNDS, adds, (unsigned long long)low, ms);
        return 1;
    }
    printf("vkatomic64: %s: ok, %u rounds of %u adds (%.0f ms)\n", c->name, ROUNDS, adds, ms);
    return 0;
}

int main(int argc, char **argv)
{
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .pApplicationName = "mach9 vkatomic64",
                              .apiVersion = VK_API_VERSION_1_3 };
    VkInstanceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                  .pApplicationInfo = &app };
    PFN_vkGetInstanceProcAddr get_instance_proc;
    PFN_vkCreateInstance create_instance;
    int status = 0;
    size_t i;
    void *loader;

    setvbuf(stdout, NULL, _IOLBF, 0);
    if (argc == 3)
    {
        char *end;
        unsigned long value = strtoul(argv[2], &end, 10);

        if (*end || !value || value % GROUP_SIZE || value > (1ul << 24))
            argc = 0;
        adds = (uint32_t)value;
    }
    if (argc != 2 && argc != 3)
    {
        printf("usage: %s <libvulkan.1.dylib> [<adds>, a multiple of %u up to 2^24]\n", argv[0],
               GROUP_SIZE);
        return 2;
    }
    start_watchdog();
    if (!(loader = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("vkatomic64: cannot load %s: %s\n", argv[1], dlerror());
        return 2;
    }
    get_instance_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    if (!get_instance_proc)
    {
        printf("vkatomic64: %s has no vkGetInstanceProcAddr\n", argv[1]);
        return 2;
    }
    create_instance = (PFN_vkCreateInstance)get_instance_proc(NULL, "vkCreateInstance");
    if (!create_instance || !ok(create_instance(&info, NULL, &instance), "vkCreateInstance"))
        return 2;
#define LOAD(name)                                                                                 \
    if (!(name = (PFN_##name)get_instance_proc(instance, #name)))                                  \
    {                                                                                              \
        printf("vkatomic64: the loader has no %s\n", #name);                                       \
        return 2;                                                                                  \
    }
    FUNCS(LOAD)
#undef LOAD
    if (!create_device() || !create_objects()) return 2;

    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        int result = run_case(&cases[i], !i);

        if (result == 2) return 2;
        if (result == 1) status = 1;
    }
    return status;
}
