/*
 * vkatomic64lanes <libvulkan.1.dylib>: check what 64-bit atomics return
 * when every invocation of a compute dispatch targets one word, and that
 * the word ends right. KosmicKrisp runs them under a lock; when every active
 * lane of a subgroup targets one address, the subgroup takes it once and
 * applies its lanes' operations in lane order, each lane getting the value
 * the lanes before it left (#240). The cases, all with vkatomic64lanes.comp:
 *
 *   <target>-add          every invocation adds lane + 1
 *   <target>-add-loop     every invocation adds lane + 1 lane % 5 + 1
 *                         times, in a loop the lanes leave one after another
 *   <target>-add-early    every third invocation returns before its add
 *   <target>-add-partial  workgroups of 48, so each second subgroup has 16
 *                         lanes
 *   <target>-max          every invocation takes the maximum with a 64-bit
 *                         value of its own
 *   <target>-chain        workgroups of one subgroup; lane i swaps i for
 *                         i + 1, so the word counts the lanes that found
 *                         their predecessor's value
 *
 * <target> is buffer (one word for the dispatch), shared (one word per
 * workgroup) or image (texel (0, 0) of an R64_UINT image). There is no
 * shared add-early: its workgroups end with a barrier.
 *
 * Each case runs ROUNDS dispatches on a word set to 0. Vulkan gives the
 * atomics of different invocations no order, so the correctness checks
 * accept any serial order of all the operations:
 *
 * - add: the returned values, sorted, run from 0 in steps of the add that
 *   returned the one before, and the word is the last one plus its add, so
 *   every add happened once and got the value before it;
 * - max: the word is the largest value, every nonzero returned value is the
 *   value of an operation that returned less, and at most one operation
 *   with a nonzero value got 0;
 * - chain: the lanes whose swap succeeded are exactly lanes 0 to w - 1 of
 *   their workgroup, where w is the word, and every other lane got a value
 *   of at most w other than the one it compared.
 *
 * A second check pins the order KosmicKrisp's subgroup lock uses
 * (locked_group in kk_nir_lower_atomic64.c): within each subgroup and loop
 * iteration, the active lanes in lane order got what the lanes before them
 * left, and every lane of a chain found its predecessor. It is the only sign
 * that the buffer and shared cases ran that lock, not a correctness check:
 * a correct lowering that orders the lanes otherwise, such as a lock per
 * lane, fails it, and it has to change with the lowering.
 *
 * Prints a line per case. Exits with 2 as soon as a case cannot run (a
 * missing feature, a failed setup, submit or wait), else with 1 when a
 * correctness check failed, else with 3 when the order check failed, 0
 * otherwise. A watchdog ends a run that hangs with 2. Build it against the
 * Vulkan headers (-I) the loader was built with.
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

#include "vkatomic64lanes.comp.h"

#define INVOCATIONS 4096u /* per dispatch, rounded down to whole workgroups */
#define MAX_TRIPS 5u      /* MAX_TRIPS of vkatomic64lanes.comp */
#define CHAIN_GROUPS 64u  /* workgroups of the shared chain */
#define ROUNDS 4
#define WATCHDOG_SECONDS 5
#define FENCE_TIMEOUT_NS 3000000000ull

#define FUNCS(X)                                                                                   \
    X(vkEnumeratePhysicalDevices)                                                                  \
    X(vkGetPhysicalDeviceProperties)                                                               \
    X(vkGetPhysicalDeviceProperties2)                                                              \
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

/* The op, target and pattern values of vkatomic64lanes.comp. */
enum op
{
    OP_ADD,
    OP_MAX,
    OP_CHAIN,
};

enum target
{
    TARGET_BUFFER,
    TARGET_SHARED,
    TARGET_IMAGE,
};

enum pattern
{
    PATTERN_ALL,
    PATTERN_LOOP,
    PATTERN_EARLY,
};

/* The local sizes of the pipelines. */
enum size
{
    SIZE_64,
    SIZE_48,
    SIZE_32,
    SIZE_COUNT,
};

static const uint32_t local_sizes[SIZE_COUNT] = { 64, 48, 32 };

struct atomic_case
{
    const char *name;
    enum op op;
    enum target target;
    enum pattern pattern;
    enum size size;
};

static const struct atomic_case cases[] = {
    { "buffer-add", OP_ADD, TARGET_BUFFER, PATTERN_ALL, SIZE_64 },
    { "buffer-add-loop", OP_ADD, TARGET_BUFFER, PATTERN_LOOP, SIZE_64 },
    { "buffer-add-early", OP_ADD, TARGET_BUFFER, PATTERN_EARLY, SIZE_64 },
    { "buffer-add-partial", OP_ADD, TARGET_BUFFER, PATTERN_ALL, SIZE_48 },
    { "buffer-max", OP_MAX, TARGET_BUFFER, PATTERN_ALL, SIZE_64 },
    { "buffer-chain", OP_CHAIN, TARGET_BUFFER, PATTERN_ALL, SIZE_32 },
    { "shared-add", OP_ADD, TARGET_SHARED, PATTERN_ALL, SIZE_64 },
    { "shared-add-loop", OP_ADD, TARGET_SHARED, PATTERN_LOOP, SIZE_64 },
    { "shared-add-partial", OP_ADD, TARGET_SHARED, PATTERN_ALL, SIZE_48 },
    { "shared-max", OP_MAX, TARGET_SHARED, PATTERN_ALL, SIZE_64 },
    { "shared-chain", OP_CHAIN, TARGET_SHARED, PATTERN_ALL, SIZE_32 },
    { "image-add", OP_ADD, TARGET_IMAGE, PATTERN_ALL, SIZE_64 },
    { "image-add-loop", OP_ADD, TARGET_IMAGE, PATTERN_LOOP, SIZE_64 },
    { "image-add-early", OP_ADD, TARGET_IMAGE, PATTERN_EARLY, SIZE_64 },
    { "image-add-partial", OP_ADD, TARGET_IMAGE, PATTERN_ALL, SIZE_48 },
    { "image-max", OP_MAX, TARGET_IMAGE, PATTERN_ALL, SIZE_64 },
    { "image-chain", OP_CHAIN, TARGET_IMAGE, PATTERN_ALL, SIZE_32 },
};

struct buffer
{
    VkBuffer handle;
    void *map;
};

/* One atomic of a dispatch, as the host sees it after the dispatch. */
struct op_result
{
    uint32_t subgroup, lane, iteration, group;
    uint64_t data, returned;
};

static VkInstance instance;
static VkPhysicalDevice physical;
static VkPhysicalDeviceMemoryProperties memory_properties;
static VkDevice device;
static VkQueue queue;
static uint32_t queue_family;
static uint32_t subgroup_size;
static VkCommandBuffer cmd;
static VkFence fence;
static VkPipelineLayout pipeline_layout;
static VkPipeline pipelines[SIZE_COUNT];
static VkImage image;
static VkImageView view;
static VkDescriptorSet set;
static struct buffer words, results, lanes, zero, readback;
static struct op_result *ops;

static bool ok(VkResult result, const char *what)
{
    if (result == VK_SUCCESS) return true;
    printf("vkatomic64lanes: %s failed: %d\n", what, (int)result);
    return false;
}

static void watchdog(int sig)
{
    static const char message[] = "vkatomic64lanes: no result within the watchdog's limit\n";

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

/* value() of vkatomic64lanes.comp. */
static uint64_t value(uint32_t index)
{
    uint32_t h = index * 0x9e3779b1u;

    h ^= h >> 15;
    h *= 0x85ebca77u;
    h ^= h >> 13;
    return ((uint64_t)h << 32) | index;
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
        printf("vkatomic64lanes: no memory type with flags %#x in %#x\n", flags,
               req.memoryTypeBits);
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

/* A 1x1 R64_UINT storage image and its view. */
static bool create_image(void)
{
    VkImageCreateInfo info = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                               .imageType = VK_IMAGE_TYPE_2D,
                               .format = VK_FORMAT_R64_UINT,
                               .extent = { 1, 1, 1 },
                               .mipLevels = 1,
                               .arrayLayers = 1,
                               .samples = VK_SAMPLE_COUNT_1_BIT,
                               .tiling = VK_IMAGE_TILING_OPTIMAL,
                               .usage = VK_IMAGE_USAGE_STORAGE_BIT |
                                        VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                                        VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                               .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
                               .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    VkImageViewCreateInfo view_info = { .sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                                        .viewType = VK_IMAGE_VIEW_TYPE_2D,
                                        .format = VK_FORMAT_R64_UINT,
                                        .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0,
                                                              1 } };
    VkMemoryRequirements req;
    VkDeviceMemory memory;

    if (!ok(vkCreateImage(device, &info, NULL, &image), "vkCreateImage")) return false;
    vkGetImageMemoryRequirements(device, image, &req);
    view_info.image = image;
    return allocate(req, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, &memory) &&
           ok(vkBindImageMemory(device, image, memory, 0), "vkBindImageMemory") &&
           ok(vkCreateImageView(device, &view_info, NULL, &view), "vkCreateImageView");
}

static bool create_device(void)
{
    static const char *const extensions[] = { VK_EXT_SHADER_IMAGE_ATOMIC_INT64_EXTENSION_NAME };
    VkPhysicalDeviceShaderImageAtomicInt64FeaturesEXT image_int64 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_IMAGE_ATOMIC_INT64_FEATURES_EXT
    };
    VkPhysicalDeviceVulkan12Features features12 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES,
        .pNext = &image_int64
    };
    VkPhysicalDeviceVulkan13Features features13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        .pNext = &features12
    };
    VkPhysicalDeviceFeatures2 features = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                                           .pNext = &features13 };
    VkPhysicalDeviceShaderImageAtomicInt64FeaturesEXT enable_image_int64 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_IMAGE_ATOMIC_INT64_FEATURES_EXT,
        .shaderImageInt64Atomics = VK_TRUE
    };
    VkPhysicalDeviceVulkan12Features enable12 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES,
        .pNext = &enable_image_int64,
        .shaderBufferInt64Atomics = VK_TRUE,
        .shaderSharedInt64Atomics = VK_TRUE
    };
    VkPhysicalDeviceVulkan13Features enable13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        .pNext = &enable12,
        .synchronization2 = VK_TRUE
    };
    VkPhysicalDeviceFeatures2 enable = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                                         .pNext = &enable13,
                                         .features.shaderInt64 = VK_TRUE };
    VkPhysicalDeviceSubgroupProperties subgroup = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_PROPERTIES
    };
    VkPhysicalDeviceProperties2 props = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2,
                                          .pNext = &subgroup };
    float priority = 1.0f;
    VkDeviceQueueCreateInfo queue_info = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                           .queueCount = 1,
                                           .pQueuePriorities = &priority };
    VkDeviceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                                .pNext = &enable,
                                .queueCreateInfoCount = 1,
                                .pQueueCreateInfos = &queue_info,
                                .enabledExtensionCount = 1,
                                .ppEnabledExtensionNames = extensions };
    VkQueueFamilyProperties families[16];
    VkFormatProperties format;
    uint32_t count = 1, i;
    VkResult res;

    res = vkEnumeratePhysicalDevices(instance, &count, &physical);
    if ((res != VK_SUCCESS && res != VK_INCOMPLETE) || !count)
    {
        printf("vkatomic64lanes: no Vulkan device, result %d\n", (int)res);
        return false;
    }
    vkGetPhysicalDeviceProperties2(physical, &props);
    vkGetPhysicalDeviceMemoryProperties(physical, &memory_properties);
    vkGetPhysicalDeviceFeatures2(physical, &features);
    vkGetPhysicalDeviceFormatProperties(physical, VK_FORMAT_R64_UINT, &format);
    subgroup_size = subgroup.subgroupSize;
    printf("vkatomic64lanes: device %s, driverVersion %#x, subgroup size %u\n",
           props.properties.deviceName, props.properties.driverVersion, subgroup_size);
    if (!features.features.shaderInt64 || !features13.synchronization2 ||
        !features12.shaderBufferInt64Atomics || !features12.shaderSharedInt64Atomics ||
        !image_int64.shaderImageInt64Atomics)
    {
        printf("vkatomic64lanes: the device lacks shaderInt64, synchronization2, or 64-bit "
               "buffer, shared or image atomics\n");
        return false;
    }
    if (!(subgroup.supportedStages & VK_SHADER_STAGE_COMPUTE_BIT) ||
        !(subgroup.supportedOperations & VK_SUBGROUP_FEATURE_BASIC_BIT) || subgroup_size != 32)
    {
        printf("vkatomic64lanes: the cases need basic subgroup operations in compute shaders "
               "and subgroups of 32 lanes\n");
        return false;
    }
    if (!(format.optimalTilingFeatures & VK_FORMAT_FEATURE_STORAGE_IMAGE_ATOMIC_BIT))
    {
        printf("vkatomic64lanes: R64_UINT has no storage image atomics\n");
        return false;
    }
    count = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(physical, &count, families);
    for (i = 0; i < count; i++)
        if (families[i].queueFlags & VK_QUEUE_COMPUTE_BIT) break;
    if (i == count)
    {
        printf("vkatomic64lanes: no queue family with compute\n");
        return false;
    }
    queue_family = queue_info.queueFamilyIndex = i;
    if (!ok(vkCreateDevice(physical, &info, NULL, &device), "vkCreateDevice")) return false;
    vkGetDeviceQueue(device, queue_family, 0, &queue);
    return true;
}

static bool create_pipelines(void)
{
    VkDescriptorSetLayoutBinding bindings[4] = {
        { 0, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL },
        { 1, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL },
        { 2, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL },
        { 3, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL },
    };
    VkDescriptorSetLayoutCreateInfo set_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        .bindingCount = 4,
        .pBindings = bindings
    };
    VkPushConstantRange push = { VK_SHADER_STAGE_COMPUTE_BIT, 0, 3 * sizeof(uint32_t) };
    VkPipelineLayoutCreateInfo layout_info = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                                               .setLayoutCount = 1,
                                               .pushConstantRangeCount = 1,
                                               .pPushConstantRanges = &push };
    VkDescriptorPoolSize sizes[2] = { { VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 3 },
                                      { VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1 } };
    VkDescriptorPoolCreateInfo pool_info = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                                             .maxSets = 1,
                                             .poolSizeCount = 2,
                                             .pPoolSizes = sizes };
    VkDescriptorSetAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                                          .descriptorSetCount = 1 };
    VkDescriptorBufferInfo buffer_infos[3] = { { words.handle, 0, VK_WHOLE_SIZE },
                                               { results.handle, 0, VK_WHOLE_SIZE },
                                               { lanes.handle, 0, VK_WHOLE_SIZE } };
    VkDescriptorImageInfo image_info = { VK_NULL_HANDLE, view, VK_IMAGE_LAYOUT_GENERAL };
    VkWriteDescriptorSet writes[4];
    VkShaderModuleCreateInfo module_info = { .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
                                             .codeSize = sizeof(vkatomic64lanes_comp),
                                             .pCode = vkatomic64lanes_comp };
    VkSpecializationMapEntry entry = { 0, 0, sizeof(uint32_t) };
    VkSpecializationInfo spec = { 1, &entry, sizeof(uint32_t), NULL };
    VkComputePipelineCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        .stage = { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                   .stage = VK_SHADER_STAGE_COMPUTE_BIT,
                   .pName = "main",
                   .pSpecializationInfo = &spec },
    };
    VkDescriptorSetLayout set_layout;
    VkDescriptorPool pool;
    uint32_t i;

    if (!ok(vkCreateDescriptorSetLayout(device, &set_info, NULL, &set_layout),
            "vkCreateDescriptorSetLayout"))
        return false;
    layout_info.pSetLayouts = &set_layout;
    alloc.pSetLayouts = &set_layout;
    if (!ok(vkCreatePipelineLayout(device, &layout_info, NULL, &pipeline_layout),
            "vkCreatePipelineLayout") ||
        !ok(vkCreateDescriptorPool(device, &pool_info, NULL, &pool), "vkCreateDescriptorPool"))
        return false;
    alloc.descriptorPool = pool;
    if (!ok(vkAllocateDescriptorSets(device, &alloc, &set), "vkAllocateDescriptorSets"))
        return false;
    for (i = 0; i < 4; i++)
        writes[i] = (VkWriteDescriptorSet){ .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                                            .dstSet = set,
                                            .dstBinding = i,
                                            .descriptorCount = 1,
                                            .descriptorType = bindings[i].descriptorType };
    writes[0].pBufferInfo = &buffer_infos[0];
    writes[1].pImageInfo = &image_info;
    writes[2].pBufferInfo = &buffer_infos[1];
    writes[3].pBufferInfo = &buffer_infos[2];
    vkUpdateDescriptorSets(device, 4, writes, 0, NULL);

    if (!ok(vkCreateShaderModule(device, &module_info, NULL, &info.stage.module),
            "vkCreateShaderModule"))
        return false;
    info.layout = pipeline_layout;
    for (i = 0; i < SIZE_COUNT; i++)
    {
        spec.pData = &local_sizes[i];
        if (!ok(vkCreateComputePipelines(device, VK_NULL_HANDLE, 1, &info, NULL, &pipelines[i]),
                "vkCreateComputePipelines"))
            return false;
    }
    vkDestroyShaderModule(device, info.stage.module, NULL);
    return true;
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

    if (!(ops = calloc(INVOCATIONS * MAX_TRIPS, sizeof(*ops))))
    {
        printf("vkatomic64lanes: out of memory\n");
        return false;
    }
    if (!ok(vkCreateCommandPool(device, &pool_info, NULL, &command_pool), "vkCreateCommandPool"))
        return false;
    cmd_info.commandPool = command_pool;
    return ok(vkAllocateCommandBuffers(device, &cmd_info, &cmd), "vkAllocateCommandBuffers") &&
           ok(vkCreateFence(device, &fence_info, NULL, &fence), "vkCreateFence") &&
           create_buffer((1 + INVOCATIONS) * sizeof(uint64_t), VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
                         &words) &&
           create_buffer(INVOCATIONS * MAX_TRIPS * sizeof(uint64_t),
                         VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, &results) &&
           create_buffer(INVOCATIONS * 2 * sizeof(uint32_t), VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
                         &lanes) &&
           create_buffer(sizeof(uint64_t), VK_BUFFER_USAGE_TRANSFER_SRC_BIT, &zero) &&
           create_buffer(sizeof(uint64_t), VK_BUFFER_USAGE_TRANSFER_DST_BIT, &readback) &&
           create_image() && create_pipelines();
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

static void to_general(void)
{
    VkImageMemoryBarrier2 barrier = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
        .dstStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
        .dstAccessMask = VK_ACCESS_2_MEMORY_READ_BIT | VK_ACCESS_2_MEMORY_WRITE_BIT,
        .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
        .newLayout = VK_IMAGE_LAYOUT_GENERAL,
        .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
        .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
        .image = image,
        .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 }
    };
    VkDependencyInfo dep = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                             .imageMemoryBarrierCount = 1,
                             .pImageMemoryBarriers = &barrier };

    vkCmdPipelineBarrier2(cmd, &dep);
}

static uint32_t groups_of(const struct atomic_case *c)
{
    if (c->op == OP_CHAIN) return c->target == TARGET_SHARED ? CHAIN_GROUPS : 1;
    return INVOCATIONS / local_sizes[c->size];
}

/* One dispatch of <c> on a word set to 0, the word read back. */
static bool run_round(const struct atomic_case *c, bool first)
{
    VkCommandBufferBeginInfo begin = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                       .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VkSubmitInfo submit = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                            .commandBufferCount = 1,
                            .pCommandBuffers = &cmd };
    VkBufferImageCopy region = { .imageSubresource = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1 },
                                 .imageExtent = { 1, 1, 1 } };
    const uint32_t push[3] = { c->op, c->target, c->pattern };

    memset(words.map, 0, (1 + INVOCATIONS) * sizeof(uint64_t));
    memset(results.map, 0xff, INVOCATIONS * MAX_TRIPS * sizeof(uint64_t));
    memset(readback.map, 0xff, sizeof(uint64_t));
    if (!ok(vkResetCommandBuffer(cmd, 0), "vkResetCommandBuffer") ||
        !ok(vkBeginCommandBuffer(cmd, &begin), "vkBeginCommandBuffer"))
        return false;
    if (first) to_general();
    barrier(VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
            VK_ACCESS_2_MEMORY_READ_BIT | VK_ACCESS_2_MEMORY_WRITE_BIT,
            VK_PIPELINE_STAGE_2_COPY_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT);
    vkCmdCopyBufferToImage(cmd, zero.handle, image, VK_IMAGE_LAYOUT_GENERAL, 1, &region);
    barrier(VK_PIPELINE_STAGE_2_COPY_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
            VK_ACCESS_2_SHADER_STORAGE_READ_BIT | VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT);
    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipelines[c->size]);
    vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline_layout, 0, 1, &set, 0,
                            NULL);
    vkCmdPushConstants(cmd, pipeline_layout, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof(push), push);
    vkCmdDispatch(cmd, groups_of(c), 1, 1);
    barrier(VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT,
            VK_PIPELINE_STAGE_2_COPY_BIT | VK_PIPELINE_STAGE_2_HOST_BIT,
            VK_ACCESS_2_TRANSFER_READ_BIT | VK_ACCESS_2_HOST_READ_BIT);
    vkCmdCopyImageToBuffer(cmd, image, VK_IMAGE_LAYOUT_GENERAL, readback.handle, 1, &region);
    barrier(VK_PIPELINE_STAGE_2_COPY_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_2_HOST_BIT, VK_ACCESS_2_HOST_READ_BIT);
    return ok(vkEndCommandBuffer(cmd), "vkEndCommandBuffer") &&
           ok(vkResetFences(device, 1, &fence), "vkResetFences") &&
           ok(vkQueueSubmit(queue, 1, &submit, fence), "vkQueueSubmit") &&
           ok(vkWaitForFences(device, 1, &fence, VK_TRUE, FENCE_TIMEOUT_NS), "vkWaitForFences");
}

/* The word a group of atomics ends in: the dispatch's, or a workgroup's. */
static uint64_t final_word(const struct atomic_case *c, uint32_t group)
{
    if (c->target == TARGET_SHARED) return ((const uint64_t *)words.map)[1 + group];
    if (c->target == TARGET_IMAGE) return *(const uint64_t *)readback.map;
    return *(const uint64_t *)words.map;
}

/* The atomics of the last dispatch of <c>, as vkatomic64lanes.comp ran
 * them; returns their count. */
static uint32_t collect(const struct atomic_case *c)
{
    const uint32_t local = local_sizes[c->size];
    const uint32_t invocations = groups_of(c) * local;
    const uint64_t *returned = results.map;
    const uint32_t *lane_of = lanes.map;
    uint32_t i, trip, count = 0;

    for (i = 0; i < invocations; i++)
    {
        const uint32_t lane = lane_of[2 * i + 1];
        const uint32_t trips = c->pattern == PATTERN_LOOP ? lane % MAX_TRIPS + 1 : 1;
        if (c->pattern == PATTERN_EARLY && i % 3 == 0) continue;
        for (trip = 0; trip < trips; trip++)
        {
            struct op_result *op = &ops[count++];
            op->subgroup = lane_of[2 * i];
            op->lane = lane;
            op->iteration = trip;
            op->group = c->target == TARGET_SHARED ? i / local : 0;
            op->data = c->op == OP_MAX     ? value(i)
                       : c->op == OP_CHAIN ? (c->target == TARGET_SHARED ? i % local : i) + 1
                                           : (uint64_t)lane + 1;
            op->returned = returned[i * MAX_TRIPS + trip];
        }
    }
    return count;
}

static int by_returned(const void *a, const void *b)
{
    const struct op_result *x = a, *y = b;

    if (x->group != y->group) return x->group < y->group ? -1 : 1;
    return x->returned < y->returned ? -1 : x->returned > y->returned;
}

static int by_lane(const void *a, const void *b)
{
    const struct op_result *x = a, *y = b;

    if (x->subgroup != y->subgroup) return x->subgroup < y->subgroup ? -1 : 1;
    if (x->iteration != y->iteration) return x->iteration < y->iteration ? -1 : 1;
    return x->lane < y->lane ? -1 : x->lane > y->lane;
}

static int by_data(const void *a, const void *b)
{
    const struct op_result *x = a, *y = b;

    if (x->group != y->group) return x->group < y->group ? -1 : 1;
    return x->data < y->data ? -1 : x->data > y->data;
}

/* The correctness checks of the header on the atomics of one dispatch, which
 * hold for any serial order; prints what is wrong into <what>. */
static bool check_correct(const struct atomic_case *c, uint32_t count, char *what, size_t size)
{
    uint32_t i, start;

    qsort(ops, count, sizeof(*ops), c->op == OP_MAX ? by_data : by_returned);
    for (start = 0; start < count; start = i)
    {
        const uint32_t group = ops[start].group;
        const uint64_t word = final_word(c, group);
        uint64_t expect = 0;
        uint32_t end, zeros = 0;

        for (end = start; end < count && ops[end].group == group; end++)
            ;
        for (i = start; i < end; i++)
        {
            const struct op_result *op = &ops[i];

            if (c->op == OP_ADD)
            {
                if (op->returned != expect)
                {
                    snprintf(what, size, "an add returned %llu where %llu was due",
                             (unsigned long long)op->returned, (unsigned long long)expect);
                    return false;
                }
                expect += op->data;
            }
            else if (c->op == OP_MAX)
            {
                struct op_result key = { .group = group, .data = op->returned };
                const struct op_result *before;

                if (op->data > expect) expect = op->data;
                if (!op->returned)
                {
                    if (op->data && ++zeros > 1)
                    {
                        snprintf(what, size, "two maximums with values got 0");
                        return false;
                    }
                    continue;
                }
                before = bsearch(&key, &ops[start], end - start, sizeof(*ops), by_data);
                if (!before || before->returned >= op->returned)
                {
                    snprintf(what, size, "a maximum got %llu, %s",
                             (unsigned long long)op->returned,
                             before ? "the value of an operation that got as much"
                                    : "which no operation had");
                    return false;
                }
            }
            else if (op->returned + 1 == op->data)
            {
                /* A swap that succeeded: its lane found its predecessor's
                 * value. */
                if (op->returned >= word)
                {
                    snprintf(what, size, "lane %llu swapped, but the word is %llu",
                             (unsigned long long)op->returned, (unsigned long long)word);
                    return false;
                }
                expect++;
            }
            else if (op->returned > word)
            {
                /* A swap that failed got the word as it was then, which the
                 * swaps only ever raise to <word>. */
                snprintf(what, size, "the lane that compared %llu got %llu, above the word %llu",
                         (unsigned long long)op->data - 1, (unsigned long long)op->returned,
                         (unsigned long long)word);
                return false;
            }
        }
        if (word != expect)
        {
            snprintf(what, size, "word %u is %llu, not %llu", group, (unsigned long long)word,
                     (unsigned long long)expect);
            return false;
        }
    }
    return true;
}

/* The order check of the header: KosmicKrisp's subgroup lock applies the
 * operations of a subgroup in lane order. */
static bool check_order(const struct atomic_case *c, uint32_t count, char *what, size_t size)
{
    uint32_t i;

    if (c->op == OP_CHAIN)
    {
        qsort(ops, count, sizeof(*ops), by_returned);
        for (i = 0; i < count; i++)
            if (ops[i].returned + 1 != ops[i].data)
            {
                snprintf(what, size, "the lane that compared %llu got %llu",
                         (unsigned long long)ops[i].data - 1,
                         (unsigned long long)ops[i].returned);
                return false;
            }
    }
    qsort(ops, count, sizeof(*ops), by_lane);
    for (i = 1; i < count; i++)
    {
        const struct op_result *prev = &ops[i - 1], *op = &ops[i];
        uint64_t expect;

        if (op->subgroup != prev->subgroup || op->iteration != prev->iteration) continue;
        expect = c->op == OP_ADD   ? prev->returned + prev->data
                 : c->op == OP_MAX ? (prev->data > prev->returned ? prev->data : prev->returned)
                                   : prev->returned + 1;
        if (op->returned != expect)
        {
            snprintf(what, size,
                     "subgroup %u, iteration %u: lane %u got %llu after lane %u got %llu, "
                     "not %llu",
                     op->subgroup, op->iteration, op->lane, (unsigned long long)op->returned,
                     prev->lane, (unsigned long long)prev->returned, (unsigned long long)expect);
            return false;
        }
    }
    return true;
}

/* Runs the rounds of one case; returns 2 when a round could not run, 1 when
 * a correctness check failed, 3 when only the order check did, else 0. */
static int run_case(const struct atomic_case *c, bool first)
{
    struct timespec start, end;
    uint32_t count = 0;
    char what[256] = "", order[256] = "";
    unsigned round, unordered = 0;
    double ms;

    clock_gettime(CLOCK_MONOTONIC, &start);
    for (round = 0; round < ROUNDS; round++)
    {
        if (!run_round(c, first && !round)) return 2;
        count = collect(c);
        if (!check_correct(c, count, what, sizeof(what)))
        {
            printf("vkatomic64lanes: %s: wrong in round %u of %u: %s\n", c->name, round + 1,
                   ROUNDS, what);
            return 1;
        }
        if (!check_order(c, count, what, sizeof(what)) && !unordered++)
            snprintf(order, sizeof(order), "%s", what);
    }
    clock_gettime(CLOCK_MONOTONIC, &end);
    ms = (double)(end.tv_sec - start.tv_sec) * 1e3 + (double)(end.tv_nsec - start.tv_nsec) / 1e6;
    if (unordered)
    {
        printf("vkatomic64lanes: %s: fits a serial order, but not the subgroup lock's lane order "
               "in %u of %u rounds: %s\n",
               c->name, unordered, ROUNDS, order);
        return 3;
    }
    printf("vkatomic64lanes: %s: ok, %u rounds of %u atomics (%.0f ms)\n", c->name, ROUNDS, count,
           ms);
    return 0;
}

int main(int argc, char **argv)
{
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .pApplicationName = "mach9 vkatomic64lanes",
                              .apiVersion = VK_API_VERSION_1_3 };
    VkInstanceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                  .pApplicationInfo = &app };
    PFN_vkGetInstanceProcAddr get_instance_proc;
    PFN_vkCreateInstance create_instance;
    int status = 0;
    size_t i;
    void *loader;

    setvbuf(stdout, NULL, _IOLBF, 0);
    if (argc != 2)
    {
        printf("usage: %s <libvulkan.1.dylib>\n", argv[0]);
        return 2;
    }
    start_watchdog();
    if (!(loader = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("vkatomic64lanes: cannot load %s: %s\n", argv[1], dlerror());
        return 2;
    }
    get_instance_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    if (!get_instance_proc)
    {
        printf("vkatomic64lanes: %s has no vkGetInstanceProcAddr\n", argv[1]);
        return 2;
    }
    create_instance = (PFN_vkCreateInstance)get_instance_proc(NULL, "vkCreateInstance");
    if (!create_instance || !ok(create_instance(&info, NULL, &instance), "vkCreateInstance"))
        return 2;
#define LOAD(name)                                                                                 \
    if (!(name = (PFN_##name)get_instance_proc(instance, #name)))                                  \
    {                                                                                              \
        printf("vkatomic64lanes: the loader has no %s\n", #name);                                  \
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
        if (result == 3 && !status) status = 3;
    }
    return status;
}
