/* GPU time of KosmicKrisp on the patterns behind the Enshrouded gap (#36):
 * indirect-count draws with a large maxDrawCount (#61), buffer loads under
 * robust buffer access (#62) and chains of compute barriers (#63). It runs
 * natively, outside Wine, on the Vulkan loader it is given, which takes the
 * driver from VK_DRIVER_FILES. Build and run it with kk.sh next to it:
 *
 *     bench/kk.sh [<bundle or MESA_DIST>] [--rounds <n>] [--case <name>]...
 *
 * The stack defaults to dist/wine. A MESA_DIST of bin/build-mesa.sh compares
 * two Mesa builds without a bundle step. --case runs only the named cases;
 * the default runs all.
 *
 * Each case has arms that differ in one setting, such as the robustness
 * features of the device. An arm records a fixed load and then its work
 * between two vkCmdWriteTimestamp into one command buffer, submits it and
 * waits for it; the GPU time is the difference times timestampPeriod, which
 * leaves the load out. Every round runs each arm once,
 * starting with another arm each round, after untimed warm-up rounds. It
 * prints one line per arm: the case, its settings, and the median,
 * quartiles, minimum and maximum over the rounds in ms. Before the timed
 * rounds each arm runs once with a check of its result (fragments drawn,
 * values computed); an arm that fails it prints FAIL and is not timed, and
 * the exit status is 1. A draw arm's check counts its fragments with an
 * atomic in the fragment shader of a pipeline of its own, since an occlusion
 * query around the draws counted only those of one indirect-count call.
 *
 * | Case | Arms | What it times |
 * | --- | --- | --- |
 * | draw-indexed-count | robustness off, robustBufferAccess, robustBufferAccess2; maxDrawCount 4096 and 32 | CALLS vkCmdDrawIndexedIndirectCount of COUNT one-triangle draws, 16-bit indices at offset 0 |
 * | draw-indexed-count-unroll | robustness off, robustBufferAccess2, the pipelineRobustness feature; maxDrawCount 4096 and 32 | the same with the index buffer bound at offset 2, which the last two arms unroll |
 * | draw-count | robustness off; maxDrawCount 4096 and 32 | CALLS vkCmdDrawIndirectCount, the same counts |
 * | draw-indexed-count-mixed | robustBufferAccess2; maxDrawCount 4096 and 32 | the calls of draw-indexed-count and of draw-indexed-count-unroll in turn, in one command buffer: the index buffer bound at offset 0 for every other call and at offset 2 for the rest |
 * | draw-indexed-count-dense | robustness off; maxDrawCount 4096 and 1024 | CALLS vkCmdDrawIndexedIndirectCount whose count is the maxDrawCount, so every slot draws |
 * | draw-count-dense | the same | the same with vkCmdDrawIndirectCount |
 * | draw-indexed-count-half | the same | CALLS vkCmdDrawIndexedIndirectCount whose count is half the maxDrawCount |
 * | dispatch-loads | robustness off, robustBufferAccess, robustBufferAccess2 | DISPATCHES single-workgroup dispatches of 32 threads, LOAD_ITERATIONS iterations of one storage and one uniform buffer load each |
 * | dispatch-loads-wide | the same | the same with four storage and two uniform buffer loads per iteration, each from a buffer of its own |
 * | barrier-chain | barriers on, off | CHAIN_LENGTH single-workgroup dispatches, with a compute-to-compute vkCmdPipelineBarrier2 between each or none |
 *
 * - A robustness setting is a device feature, so each setting has a device
 *   of its own. The pipelineRobustness arm enables only the feature; its
 *   pipelines keep the device's (disabled) robustness.
 * - What robustness costs an indexed draw depends on macOS. With
 *   robustBufferAccess2 or the pipelineRobustness feature on the device,
 *   requires_unroll_robustness (kk_cmd_draw.c) unrolls every indexed indirect
 *   draw into 32-bit indices on macOS 26 (KK_WORKAROUND_12), but on macOS 27
 *   and later only one whose index buffer address or range is not a
 *   multiple of 4 (kk_physical_device.c turns the workaround off there). So
 *   on macOS 27 the robustness arms of draw-indexed-count time like its off
 *   arm and draw-indexed-count-unroll carries the cost; on macOS 26 both
 *   cases unroll under those two arms. robustBufferAccess alone never
 *   unrolls.
 * - draw-indexed-count-mixed shows whether the paths the driver takes for
 *   the two kinds of call cost each other anything in one command buffer:
 *   without that, it reads the mean of the robustBufferAccess2 arms of the
 *   two cases it alternates.
 * - The timestamps are written outside render passes: in a render encoder
 *   KosmicKrisp hands a second timestamp of the same stage the first one's
 *   value. Outside one, it writes each after a one-thread dispatch of its own
 *   that waits for the work before it.
 * - The GPU lowers its performance state under light load and climbs back
 *   over about 16 ms of work, so without the load an arm's time depends on
 *   the arms before it: after ten draw arms of maxDrawCount 32 in a row, an
 *   arm of maxDrawCount 4096 took twice as long. The load, LOAD_DISPATCHES
 *   dispatches of LOAD_GROUPS workgroups of loads.comp, runs before the first
 *   timestamp in the run's command buffer, so the interval starts with
 *   the GPU in its highest state and nothing idle in between. The first
 *   timestamp waits for the work before it, which keeps the load out of the
 *   interval. A pause of 20 ms or more before a run still moves its time by
 *   up to 0.3 ms, since the GPU climbs more slowly after one; the rounds
 *   have no pause.
 * - Compare the difference between arms of one invocation, not absolute
 *   medians across invocations: the absolute times of one build move between
 *   invocations while the differences stay put. For a before and after,
 *   alternate invocations of the two stacks with the same --case selection.
 *   This is not the performance state, and the load does not remove it: the
 *   dispatch-loads arms read 0.045 ms more in some invocations than in
 *   others with the GPU in its highest state in both.
 * - The compute cases put both timestamps into one compute encoder. A Mesa
 *   without "kk: Write compute timestamps with precise granularity" (#115)
 *   gives them one value, and the run stops with FAIL.
 * - KosmicKrisp resolves each timestamp when its Metal encoder ends, at
 *   about 27 us of GPU time (#117). In the draw cases the first timestamp's
 *   encoder ends when rendering begins, so that resolve falls inside the
 *   interval: a constant of the case, the same in each arm.
 * - MESA_KK_DEBUG=msl prints the MSL of each arm's shaders as the driver
 *   compiles them, for #62.
 *
 * The SPIR-V comes from the GLSL in shaders/ (shaders/generate.sh). The
 * Vulkan headers are those build-mesa.sh builds the loader with. */

#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>

#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "shaders/chain.comp.h"
#include "shaders/count.frag.h"
#include "shaders/draw.frag.h"
#include "shaders/draw.vert.h"
#include "shaders/loads.comp.h"
#include "shaders/loads_wide.comp.h"

/* The draw cases. Enshrouded issues 13 robust indexed indirect-count calls
 * per frame with 1,024 slots or more, and 125,058 slots in all, 99.3 % of them
 * empty (#61); CALLS * MAX_DRAWS slots with COUNT draws each is 131,072 slots,
 * 99.2 % empty. */
#define CALLS 32
#define MAX_DRAWS 4096
#define COUNT 32
/* The smaller maxDrawCount of the dense cases: where KosmicKrisp starts to
 * run a call from an indirect command buffer (#61). */
#define DENSE_DRAWS 1024
/* The render target, which each draw's triangle covers. */
#define TARGET_SIZE 16
/* 161 single-workgroup dispatches run 2.55x slower on KosmicKrisp than on
 * MoltenVK in a replay of the game (#62). */
#define DISPATCHES 161
#define LOAD_ITERATIONS 1024
/* vec4s in each storage buffer of dispatch-loads, a power of two. */
#define SOURCE_VEC4S 16384
#define CHAIN_LENGTH 256
/* The bindings of loads_wide.comp, the most of any shader here. */
#define MAX_BINDINGS 7
/* The load before each timed interval: about 29 ms of GPU time on an M5 Max,
 * against the 16 ms of work the GPU took from idle to its highest
 * performance state. */
#define LOAD_DISPATCHES 256
#define LOAD_GROUPS 1024
#define WARMUP_ROUNDS 3
#define DEFAULT_ROUNDS 20

#define INSTANCE_FUNCS(X)                                                                          \
    X(vkDestroyInstance)                                                                           \
    X(vkEnumeratePhysicalDevices)                                                                  \
    X(vkGetPhysicalDeviceProperties)                                                               \
    X(vkGetPhysicalDeviceProperties2)                                                              \
    X(vkGetPhysicalDeviceMemoryProperties)                                                         \
    X(vkGetPhysicalDeviceQueueFamilyProperties)                                                    \
    X(vkCreateDevice)                                                                              \
    X(vkDestroyDevice)                                                                             \
    X(vkGetDeviceQueue)                                                                            \
    X(vkDeviceWaitIdle)                                                                            \
    X(vkCreateBuffer)                                                                              \
    X(vkDestroyBuffer)                                                                             \
    X(vkGetBufferMemoryRequirements)                                                               \
    X(vkBindBufferMemory)                                                                          \
    X(vkCreateImage)                                                                               \
    X(vkDestroyImage)                                                                              \
    X(vkGetImageMemoryRequirements)                                                                \
    X(vkBindImageMemory)                                                                           \
    X(vkCreateImageView)                                                                           \
    X(vkDestroyImageView)                                                                          \
    X(vkAllocateMemory)                                                                            \
    X(vkFreeMemory)                                                                                \
    X(vkMapMemory)                                                                                 \
    X(vkCreateShaderModule)                                                                        \
    X(vkDestroyShaderModule)                                                                       \
    X(vkCreateDescriptorSetLayout)                                                                 \
    X(vkDestroyDescriptorSetLayout)                                                                \
    X(vkCreatePipelineLayout)                                                                      \
    X(vkDestroyPipelineLayout)                                                                     \
    X(vkCreateComputePipelines)                                                                    \
    X(vkCreateGraphicsPipelines)                                                                   \
    X(vkDestroyPipeline)                                                                           \
    X(vkCreateDescriptorPool)                                                                      \
    X(vkDestroyDescriptorPool)                                                                     \
    X(vkAllocateDescriptorSets)                                                                    \
    X(vkUpdateDescriptorSets)                                                                      \
    X(vkCreateCommandPool)                                                                         \
    X(vkDestroyCommandPool)                                                                        \
    X(vkAllocateCommandBuffers)                                                                    \
    X(vkResetCommandBuffer)                                                                        \
    X(vkBeginCommandBuffer)                                                                        \
    X(vkEndCommandBuffer)                                                                          \
    X(vkCreateFence)                                                                               \
    X(vkDestroyFence)                                                                              \
    X(vkResetFences)                                                                               \
    X(vkWaitForFences)                                                                             \
    X(vkQueueSubmit)                                                                               \
    X(vkCreateQueryPool)                                                                           \
    X(vkDestroyQueryPool)                                                                          \
    X(vkResetQueryPool)                                                                            \
    X(vkGetQueryPoolResults)                                                                       \
    X(vkCmdWriteTimestamp)                                                                         \
    X(vkCmdBindPipeline)                                                                           \
    X(vkCmdBindDescriptorSets)                                                                     \
    X(vkCmdPushConstants)                                                                          \
    X(vkCmdDispatch)                                                                               \
    X(vkCmdPipelineBarrier2)                                                                       \
    X(vkCmdBeginRendering)                                                                         \
    X(vkCmdEndRendering)                                                                           \
    X(vkCmdSetViewport)                                                                            \
    X(vkCmdSetScissor)                                                                             \
    X(vkCmdBindVertexBuffers)                                                                      \
    X(vkCmdBindIndexBuffer)                                                                        \
    X(vkCmdDrawIndirectCount)                                                                      \
    X(vkCmdDrawIndexedIndirectCount)

#define DECLARE(name) static PFN_##name name;
INSTANCE_FUNCS(DECLARE)
#undef DECLARE
static PFN_vkGetInstanceProcAddr get_instance_proc;
static PFN_vkCreateInstance create_instance;

enum robustness
{
    ROBUST_OFF,
    ROBUST_RBA,
    ROBUST_RBA2,
    ROBUST_PIPELINE,
    ROBUST_COUNT,
};

static const char *const robustness_names[ROBUST_COUNT] = {
    "off",
    "robustBufferAccess",
    "robustBufferAccess2",
    "pipelineRobustness",
};

enum kind
{
    KIND_INDEXED,
    KIND_NONINDEXED,
    KIND_LOADS,
    KIND_CHAIN,
};

struct buffer
{
    VkBuffer buffer;
    VkDeviceMemory memory;
    void *map;
};

/* A device with one robustness setting, and what every arm on it shares. */
struct device
{
    bool created, failed;
    VkDevice device;
    VkQueue queue;
    VkCommandPool pool;
    VkCommandBuffer cmd;
    VkFence fence;
    VkQueryPool timestamps;
    VkDescriptorPool descriptors;
};

struct arm
{
    const char *name;
    enum kind kind;
    enum robustness robustness;
    uint32_t max_draws;
    uint32_t index_offset;
    /* draw-indexed-count-mixed: every second call binds the index buffer at
     * index_offset, the others at 0. */
    bool mixed;
    /* The dense and half cases: the draws the count buffer lets run in each
     * call; COUNT when 0. */
    uint32_t draws;
    bool barriers;
    /* dispatch-loads-wide: loads_wide.comp instead of loads.comp. */
    bool wide;

    bool selected, ready, failed;
    struct device *dev;
    VkPipelineLayout layout;
    VkDescriptorSetLayout set_layout;
    VkDescriptorSet set;
    VkPipeline pipeline;
    VkPipeline check_pipeline;
    VkImage image;
    VkDeviceMemory image_memory;
    VkImageView view;
    /* result: the fragment counter of a draw arm's check, or what a compute
     * arm writes. */
    struct buffer vertices, indices, indirect, count, sources[4], weights[2], result;
    double *samples;
};

#define DRAW(case_name, draw_kind, robust, max, offset)                                            \
    { .name = case_name, .kind = draw_kind, .robustness = robust, .max_draws = max,                \
      .index_offset = offset }
#define MIXED(max)                                                                                 \
    { .name = "draw-indexed-count-mixed", .kind = KIND_INDEXED, .robustness = ROBUST_RBA2,         \
      .max_draws = max, .index_offset = 2, .mixed = true }
#define FILLED(case_name, draw_kind, max, count)                                                   \
    { .name = case_name, .kind = draw_kind, .robustness = ROBUST_OFF, .max_draws = max,            \
      .draws = count }
#define LOADS(case_name, robust, is_wide)                                                          \
    { .name = case_name, .kind = KIND_LOADS, .robustness = robust, .wide = is_wide }
#define CHAIN(on) { .name = "barrier-chain", .kind = KIND_CHAIN, .barriers = on }

static struct arm arms[] = {
    DRAW("draw-indexed-count", KIND_INDEXED, ROBUST_OFF, MAX_DRAWS, 0),
    DRAW("draw-indexed-count", KIND_INDEXED, ROBUST_OFF, COUNT, 0),
    DRAW("draw-indexed-count", KIND_INDEXED, ROBUST_RBA, MAX_DRAWS, 0),
    DRAW("draw-indexed-count", KIND_INDEXED, ROBUST_RBA, COUNT, 0),
    DRAW("draw-indexed-count", KIND_INDEXED, ROBUST_RBA2, MAX_DRAWS, 0),
    DRAW("draw-indexed-count", KIND_INDEXED, ROBUST_RBA2, COUNT, 0),
    DRAW("draw-indexed-count-unroll", KIND_INDEXED, ROBUST_OFF, MAX_DRAWS, 2),
    DRAW("draw-indexed-count-unroll", KIND_INDEXED, ROBUST_OFF, COUNT, 2),
    DRAW("draw-indexed-count-unroll", KIND_INDEXED, ROBUST_RBA2, MAX_DRAWS, 2),
    DRAW("draw-indexed-count-unroll", KIND_INDEXED, ROBUST_RBA2, COUNT, 2),
    DRAW("draw-indexed-count-unroll", KIND_INDEXED, ROBUST_PIPELINE, MAX_DRAWS, 2),
    DRAW("draw-indexed-count-unroll", KIND_INDEXED, ROBUST_PIPELINE, COUNT, 2),
    DRAW("draw-count", KIND_NONINDEXED, ROBUST_OFF, MAX_DRAWS, 0),
    DRAW("draw-count", KIND_NONINDEXED, ROBUST_OFF, COUNT, 0),
    MIXED(MAX_DRAWS),
    MIXED(COUNT),
    FILLED("draw-indexed-count-dense", KIND_INDEXED, MAX_DRAWS, MAX_DRAWS),
    FILLED("draw-indexed-count-dense", KIND_INDEXED, DENSE_DRAWS, DENSE_DRAWS),
    FILLED("draw-count-dense", KIND_NONINDEXED, MAX_DRAWS, MAX_DRAWS),
    FILLED("draw-count-dense", KIND_NONINDEXED, DENSE_DRAWS, DENSE_DRAWS),
    FILLED("draw-indexed-count-half", KIND_INDEXED, MAX_DRAWS, MAX_DRAWS / 2),
    FILLED("draw-indexed-count-half", KIND_INDEXED, DENSE_DRAWS, DENSE_DRAWS / 2),
    LOADS("dispatch-loads", ROBUST_OFF, false),
    LOADS("dispatch-loads", ROBUST_RBA, false),
    LOADS("dispatch-loads", ROBUST_RBA2, false),
    LOADS("dispatch-loads-wide", ROBUST_OFF, true),
    LOADS("dispatch-loads-wide", ROBUST_RBA, true),
    LOADS("dispatch-loads-wide", ROBUST_RBA2, true),
    CHAIN(true),
    CHAIN(false),
};
#define ARM_COUNT (sizeof(arms) / sizeof(arms[0]))

/* The load each device runs before the timed interval of its arms: the
 * shader of dispatch-loads, LOAD_DISPATCHES dispatches of LOAD_GROUPS
 * workgroups. Every workgroup writes the same sums to the same place. */
static struct arm loads[ROBUST_COUNT] = {
    LOADS("load", ROBUST_OFF, false),
    LOADS("load", ROBUST_RBA, false),
    LOADS("load", ROBUST_RBA2, false),
    LOADS("load", ROBUST_PIPELINE, false),
};

static VkInstance instance;
static VkPhysicalDevice physical;
static VkPhysicalDeviceMemoryProperties memory_properties;
static uint32_t queue_family;
static uint64_t timestamp_mask;
static double timestamp_period;
static struct device devices[ROBUST_COUNT];

/* Whether <result> is VK_SUCCESS, after a line naming <what> when it is not. */
static bool ok(VkResult result, const char *what)
{
    if (result == VK_SUCCESS) return true;
    printf("kk: %s failed, result %d\n", what, (int)result);
    return false;
}

/* The draws of each call of a draw arm that run. */
static uint32_t draw_count(const struct arm *arm)
{
    return arm->draws ? arm->draws : COUNT;
}

static void settings(const struct arm *arm, char *out, size_t size)
{
    switch (arm->kind)
    {
    case KIND_INDEXED:
        snprintf(out, size, "robustness=%s calls=%u max=%u count=%u index=u16+%s%u",
                 robustness_names[arm->robustness], CALLS, arm->max_draws, draw_count(arm),
                 arm->mixed ? "0/" : "", arm->index_offset);
        break;
    case KIND_NONINDEXED:
        snprintf(out, size, "robustness=%s calls=%u max=%u count=%u",
                 robustness_names[arm->robustness], CALLS, arm->max_draws, draw_count(arm));
        break;
    case KIND_LOADS:
        snprintf(out, size, "robustness=%s dispatches=%u iterations=%u",
                 robustness_names[arm->robustness], DISPATCHES, LOAD_ITERATIONS);
        break;
    case KIND_CHAIN:
        snprintf(out, size, "barriers=%s dispatches=%u", arm->barriers ? "on" : "off",
                 CHAIN_LENGTH);
        break;
    }
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

/* A mapped buffer of host-coherent memory, filled with <data> when given. */
static bool create_buffer(struct device *dev, VkDeviceSize size, VkBufferUsageFlags usage,
                          const void *data, struct buffer *out)
{
    VkBufferCreateInfo info = { .sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
                                .size = size,
                                .usage = usage,
                                .sharingMode = VK_SHARING_MODE_EXCLUSIVE };
    VkMemoryAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
    VkMemoryRequirements req;
    int type;

    if (!ok(vkCreateBuffer(dev->device, &info, NULL, &out->buffer), "vkCreateBuffer"))
        return false;
    vkGetBufferMemoryRequirements(dev->device, out->buffer, &req);
    type = find_memory_type(req.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT |
                                                    VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
    if (type < 0)
    {
        printf("kk: no host-coherent memory type for a buffer\n");
        return false;
    }
    alloc.allocationSize = req.size;
    alloc.memoryTypeIndex = (uint32_t)type;
    if (!ok(vkAllocateMemory(dev->device, &alloc, NULL, &out->memory), "vkAllocateMemory") ||
        !ok(vkBindBufferMemory(dev->device, out->buffer, out->memory, 0), "vkBindBufferMemory") ||
        !ok(vkMapMemory(dev->device, out->memory, 0, VK_WHOLE_SIZE, 0, &out->map), "vkMapMemory"))
        return false;
    if (data)
        memcpy(out->map, data, size);
    else
        memset(out->map, 0, size);
    return true;
}

static void destroy_buffer(struct device *dev, struct buffer *buffer)
{
    if (buffer->buffer) vkDestroyBuffer(dev->device, buffer->buffer, NULL);
    if (buffer->memory) vkFreeMemory(dev->device, buffer->memory, NULL);
    memset(buffer, 0, sizeof(*buffer));
}

/* Creates the device of <robustness> with every feature the cases use. */
static bool create_device(struct device *dev, enum robustness robustness)
{
    static const float priority = 1.0f;
    VkPhysicalDevicePipelineRobustnessFeaturesEXT pipeline_robustness = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_ROBUSTNESS_FEATURES_EXT,
        .pipelineRobustness = VK_TRUE,
    };
    VkPhysicalDeviceRobustness2FeaturesEXT robustness2 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_FEATURES_EXT,
        .robustBufferAccess2 = VK_TRUE,
    };
    VkPhysicalDeviceVulkan13Features features13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        .synchronization2 = VK_TRUE,
        .dynamicRendering = VK_TRUE,
    };
    VkPhysicalDeviceVulkan12Features features12 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES,
        .pNext = &features13,
        .drawIndirectCount = VK_TRUE,
        .hostQueryReset = VK_TRUE,
    };
    VkPhysicalDeviceFeatures2 features = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
        .pNext = &features12,
        .features.robustBufferAccess = robustness == ROBUST_RBA || robustness == ROBUST_RBA2,
        .features.fragmentStoresAndAtomics = VK_TRUE,
    };
    VkDeviceQueueCreateInfo queue = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                      .queueFamilyIndex = queue_family,
                                      .queueCount = 1,
                                      .pQueuePriorities = &priority };
    const char *extension = NULL;
    VkDeviceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                                .pNext = &features,
                                .queueCreateInfoCount = 1,
                                .pQueueCreateInfos = &queue };
    VkCommandPoolCreateInfo pool = { .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
                                     .flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
                                     .queueFamilyIndex = queue_family };
    VkCommandBufferAllocateInfo cmd = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                                        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
                                        .commandBufferCount = 1 };
    VkFenceCreateInfo fence = { .sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
    VkQueryPoolCreateInfo timestamps = { .sType = VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO,
                                         .queryType = VK_QUERY_TYPE_TIMESTAMP,
                                         .queryCount = 2 };
    VkDescriptorPoolSize sizes[] = {
        { VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, MAX_BINDINGS * (ARM_COUNT + 1) },
        { VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, MAX_BINDINGS * (ARM_COUNT + 1) },
    };
    /* The arms and the device's load. */
    VkDescriptorPoolCreateInfo descriptors = { .sType =
                                                   VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                                               .maxSets = ARM_COUNT + 1,
                                               .poolSizeCount = 2,
                                               .pPoolSizes = sizes };

    if (robustness == ROBUST_RBA2)
    {
        features13.pNext = &robustness2;
        extension = VK_EXT_ROBUSTNESS_2_EXTENSION_NAME;
    }
    else if (robustness == ROBUST_PIPELINE)
    {
        features13.pNext = &pipeline_robustness;
        extension = VK_EXT_PIPELINE_ROBUSTNESS_EXTENSION_NAME;
    }
    if (extension)
    {
        info.enabledExtensionCount = 1;
        info.ppEnabledExtensionNames = &extension;
    }

    dev->created = true;
    if (!ok(vkCreateDevice(physical, &info, NULL, &dev->device), "vkCreateDevice"))
    {
        printf("kk: no device with robustness=%s\n", robustness_names[robustness]);
        dev->failed = true;
        return false;
    }
    vkGetDeviceQueue(dev->device, queue_family, 0, &dev->queue);
    if (!ok(vkCreateCommandPool(dev->device, &pool, NULL, &dev->pool), "vkCreateCommandPool"))
        goto fail;
    cmd.commandPool = dev->pool;
    if (!ok(vkAllocateCommandBuffers(dev->device, &cmd, &dev->cmd), "vkAllocateCommandBuffers") ||
        !ok(vkCreateFence(dev->device, &fence, NULL, &dev->fence), "vkCreateFence") ||
        !ok(vkCreateQueryPool(dev->device, &timestamps, NULL, &dev->timestamps),
            "vkCreateQueryPool") ||
        !ok(vkCreateDescriptorPool(dev->device, &descriptors, NULL, &dev->descriptors),
            "vkCreateDescriptorPool"))
        goto fail;
    return true;

fail:
    dev->failed = true;
    return false;
}

static VkShaderModule create_module(struct device *dev, const uint32_t *code, size_t size)
{
    VkShaderModuleCreateInfo info = { .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
                                      .codeSize = size,
                                      .pCode = code };
    VkShaderModule module = VK_NULL_HANDLE;

    ok(vkCreateShaderModule(dev->device, &info, NULL, &module), "vkCreateShaderModule");
    return module;
}

/* The descriptor set of an arm: <buffers> at bindings 0 to <count> - 1, the
 * arm's weights as uniform buffers and the others as storage buffers. A draw
 * arm's is the fragment counter of its check. */
static bool create_set(struct arm *arm, const struct buffer *const *buffers, uint32_t count)
{
    VkShaderStageFlags stage = arm->kind == KIND_INDEXED || arm->kind == KIND_NONINDEXED
                                   ? VK_SHADER_STAGE_FRAGMENT_BIT
                                   : VK_SHADER_STAGE_COMPUTE_BIT;
    VkDescriptorSetLayoutBinding bindings[MAX_BINDINGS];
    VkDescriptorBufferInfo infos[MAX_BINDINGS];
    VkWriteDescriptorSet writes[MAX_BINDINGS];
    VkDescriptorSetLayoutCreateInfo layout = { .sType =
                                                   VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
                                               .bindingCount = count,
                                               .pBindings = bindings };
    VkDescriptorSetAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                                          .descriptorPool = arm->dev->descriptors,
                                          .descriptorSetCount = 1,
                                          .pSetLayouts = &arm->set_layout };
    uint32_t i;

    for (i = 0; i < count; i++)
    {
        VkDescriptorType type = buffers[i] == &arm->weights[0] || buffers[i] == &arm->weights[1]
                                    ? VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER
                                    : VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
        bindings[i] = (VkDescriptorSetLayoutBinding){ .binding = i,
                                                      .descriptorType = type,
                                                      .descriptorCount = 1,
                                                      .stageFlags = stage };
        infos[i] = (VkDescriptorBufferInfo){ .buffer = buffers[i]->buffer,
                                             .range = VK_WHOLE_SIZE };
        writes[i] = (VkWriteDescriptorSet){ .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                                            .dstBinding = i,
                                            .descriptorCount = 1,
                                            .descriptorType = type,
                                            .pBufferInfo = &infos[i] };
    }
    if (!ok(vkCreateDescriptorSetLayout(arm->dev->device, &layout, NULL, &arm->set_layout),
            "vkCreateDescriptorSetLayout") ||
        !ok(vkAllocateDescriptorSets(arm->dev->device, &alloc, &arm->set),
            "vkAllocateDescriptorSets"))
        return false;
    for (i = 0; i < count; i++) writes[i].dstSet = arm->set;
    vkUpdateDescriptorSets(arm->dev->device, count, writes, 0, NULL);
    return true;
}

static bool create_compute_pipeline(struct arm *arm, const uint32_t *code, size_t size,
                                    uint32_t push_size)
{
    VkPushConstantRange push = { VK_SHADER_STAGE_COMPUTE_BIT, 0, push_size };
    VkPipelineLayoutCreateInfo layout = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                                          .setLayoutCount = 1,
                                          .pSetLayouts = &arm->set_layout,
                                          .pushConstantRangeCount = push_size ? 1 : 0,
                                          .pPushConstantRanges = &push };
    VkComputePipelineCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        .stage = { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                   .stage = VK_SHADER_STAGE_COMPUTE_BIT,
                   .pName = "main" },
    };
    bool created;

    if (!ok(vkCreatePipelineLayout(arm->dev->device, &layout, NULL, &arm->layout),
            "vkCreatePipelineLayout"))
        return false;
    if (!(info.stage.module = create_module(arm->dev, code, size))) return false;
    info.layout = arm->layout;
    created = ok(vkCreateComputePipelines(arm->dev->device, VK_NULL_HANDLE, 1, &info, NULL,
                                          &arm->pipeline),
                 "vkCreateComputePipelines");
    vkDestroyShaderModule(arm->dev->device, info.stage.module, NULL);
    return created;
}

static bool create_draw_pipeline(struct arm *arm)
{
    static const VkFormat format = VK_FORMAT_R8G8B8A8_UNORM;
    static const VkDynamicState dynamic[] = { VK_DYNAMIC_STATE_VIEWPORT,
                                              VK_DYNAMIC_STATE_SCISSOR };
    VkPipelineShaderStageCreateInfo stages[2] = {
        { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
          .stage = VK_SHADER_STAGE_VERTEX_BIT,
          .pName = "main" },
        { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
          .stage = VK_SHADER_STAGE_FRAGMENT_BIT,
          .pName = "main" },
    };
    VkVertexInputBindingDescription binding = { 0, 2 * sizeof(float),
                                                VK_VERTEX_INPUT_RATE_VERTEX };
    VkVertexInputAttributeDescription attribute = { 0, 0, VK_FORMAT_R32G32_SFLOAT, 0 };
    VkPipelineVertexInputStateCreateInfo vertex = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
        .vertexBindingDescriptionCount = 1,
        .pVertexBindingDescriptions = &binding,
        .vertexAttributeDescriptionCount = 1,
        .pVertexAttributeDescriptions = &attribute,
    };
    VkPipelineInputAssemblyStateCreateInfo assembly = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
        .topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
    };
    VkPipelineViewportStateCreateInfo viewport = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
        .viewportCount = 1,
        .scissorCount = 1,
    };
    VkPipelineRasterizationStateCreateInfo raster = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
        .polygonMode = VK_POLYGON_MODE_FILL,
        .cullMode = VK_CULL_MODE_NONE,
        .frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE,
        .lineWidth = 1.0f,
    };
    VkPipelineMultisampleStateCreateInfo multisample = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
        .rasterizationSamples = VK_SAMPLE_COUNT_1_BIT,
    };
    VkPipelineColorBlendAttachmentState blend_attachment = {
        .colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT |
                          VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT,
    };
    VkPipelineColorBlendStateCreateInfo blend = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
        .attachmentCount = 1,
        .pAttachments = &blend_attachment,
    };
    VkPipelineDynamicStateCreateInfo dynamic_state = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
        .dynamicStateCount = 2,
        .pDynamicStates = dynamic,
    };
    VkPipelineRenderingCreateInfo rendering = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO,
        .colorAttachmentCount = 1,
        .pColorAttachmentFormats = &format,
    };
    VkGraphicsPipelineCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
        .pNext = &rendering,
        .stageCount = 2,
        .pStages = stages,
        .pVertexInputState = &vertex,
        .pInputAssemblyState = &assembly,
        .pViewportState = &viewport,
        .pRasterizationState = &raster,
        .pMultisampleState = &multisample,
        .pColorBlendState = &blend,
        .pDynamicState = &dynamic_state,
    };
    VkPipelineLayoutCreateInfo layout = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                                          .setLayoutCount = 1,
                                          .pSetLayouts = &arm->set_layout };
    bool created = false;

    if (!ok(vkCreatePipelineLayout(arm->dev->device, &layout, NULL, &arm->layout),
            "vkCreatePipelineLayout"))
        return false;
    info.layout = arm->layout;
    stages[0].module = create_module(arm->dev, draw_vert, sizeof(draw_vert));
    stages[1].module = create_module(arm->dev, draw_frag, sizeof(draw_frag));
    if (stages[0].module && stages[1].module)
        created = ok(vkCreateGraphicsPipelines(arm->dev->device, VK_NULL_HANDLE, 1, &info, NULL,
                                               &arm->pipeline),
                     "vkCreateGraphicsPipelines");
    if (stages[1].module) vkDestroyShaderModule(arm->dev->device, stages[1].module, NULL);
    stages[1].module = create_module(arm->dev, count_frag, sizeof(count_frag));
    if (created && stages[1].module)
        created = ok(vkCreateGraphicsPipelines(arm->dev->device, VK_NULL_HANDLE, 1, &info, NULL,
                                               &arm->check_pipeline),
                     "vkCreateGraphicsPipelines");
    else
        created = false;
    if (stages[0].module) vkDestroyShaderModule(arm->dev->device, stages[0].module, NULL);
    if (stages[1].module) vkDestroyShaderModule(arm->dev->device, stages[1].module, NULL);
    return created;
}

/* The render target of a draw arm, left in VK_IMAGE_LAYOUT_GENERAL. */
static bool create_target(struct arm *arm)
{
    struct device *dev = arm->dev;
    VkImageCreateInfo image = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                                .imageType = VK_IMAGE_TYPE_2D,
                                .format = VK_FORMAT_R8G8B8A8_UNORM,
                                .extent = { TARGET_SIZE, TARGET_SIZE, 1 },
                                .mipLevels = 1,
                                .arrayLayers = 1,
                                .samples = VK_SAMPLE_COUNT_1_BIT,
                                .tiling = VK_IMAGE_TILING_OPTIMAL,
                                .usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
                                .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
                                .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    VkImageViewCreateInfo view = { .sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                                   .viewType = VK_IMAGE_VIEW_TYPE_2D,
                                   .format = VK_FORMAT_R8G8B8A8_UNORM,
                                   .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };
    VkImageMemoryBarrier2 barrier = { .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
                                      .dstStageMask =
                                          VK_PIPELINE_STAGE_2_COLOR_ATTACHMENT_OUTPUT_BIT,
                                      .dstAccessMask = VK_ACCESS_2_COLOR_ATTACHMENT_WRITE_BIT,
                                      .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
                                      .newLayout = VK_IMAGE_LAYOUT_GENERAL,
                                      .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                      .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                      .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0,
                                                            1 } };
    VkDependencyInfo dependency = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                    .imageMemoryBarrierCount = 1,
                                    .pImageMemoryBarriers = &barrier };
    VkCommandBufferBeginInfo begin = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                       .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VkSubmitInfo submit = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                            .commandBufferCount = 1,
                            .pCommandBuffers = &dev->cmd };
    VkMemoryAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
    VkMemoryRequirements req;
    int type;

    if (!ok(vkCreateImage(dev->device, &image, NULL, &arm->image), "vkCreateImage")) return false;
    vkGetImageMemoryRequirements(dev->device, arm->image, &req);
    if ((type = find_memory_type(req.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)) < 0)
        type = find_memory_type(req.memoryTypeBits, 0);
    alloc.allocationSize = req.size;
    alloc.memoryTypeIndex = (uint32_t)type;
    if (type < 0 ||
        !ok(vkAllocateMemory(dev->device, &alloc, NULL, &arm->image_memory), "vkAllocateMemory") ||
        !ok(vkBindImageMemory(dev->device, arm->image, arm->image_memory, 0), "vkBindImageMemory"))
        return false;
    view.image = arm->image;
    if (!ok(vkCreateImageView(dev->device, &view, NULL, &arm->view), "vkCreateImageView"))
        return false;

    barrier.image = arm->image;
    if (!ok(vkResetFences(dev->device, 1, &dev->fence), "vkResetFences") ||
        !ok(vkResetCommandBuffer(dev->cmd, 0), "vkResetCommandBuffer") ||
        !ok(vkBeginCommandBuffer(dev->cmd, &begin), "vkBeginCommandBuffer"))
        return false;
    vkCmdPipelineBarrier2(dev->cmd, &dependency);
    return ok(vkEndCommandBuffer(dev->cmd), "vkEndCommandBuffer") &&
           ok(vkQueueSubmit(dev->queue, 1, &submit, dev->fence), "vkQueueSubmit") &&
           ok(vkWaitForFences(dev->device, 1, &dev->fence, VK_TRUE, UINT64_MAX),
              "vkWaitForFences");
}

/* The size of an arm's result buffer: one vec4 or counter per lane of a
 * compute arm, the fragment counter of a draw arm. */
static size_t result_size(const struct arm *arm)
{
    switch (arm->kind)
    {
    case KIND_LOADS:
        return 32 * 4 * sizeof(float);
    case KIND_CHAIN:
        return 32 * sizeof(uint32_t);
    default:
        return sizeof(uint32_t);
    }
}

static bool setup_draw(struct arm *arm)
{
    /* A triangle that covers the whole target, so each draw of the check
     * counts TARGET_SIZE * TARGET_SIZE fragments. */
    static const float vertices[] = { -1.0f, -1.0f, 3.0f, -1.0f, -1.0f, 3.0f };
    /* Indices 0, 1, 2 at byte offset 0 and at byte offset 2. A mixed arm
     * reads the first at both offsets: 0, 1, 2 and 1, 2, 0. */
    static const uint16_t aligned[] = { 0, 1, 2, 0 };
    static const uint16_t unaligned[] = { 0, 0, 1, 2 };
    const uint32_t count = draw_count(arm);
    const struct buffer *buffers[] = { &arm->result };
    struct device *dev = arm->dev;
    bool indexed = arm->kind == KIND_INDEXED;
    uint32_t stride = indexed ? sizeof(VkDrawIndexedIndirectCommand)
                              : sizeof(VkDrawIndirectCommand);
    uint8_t *slots;
    uint32_t i;

    if (!create_buffer(dev, result_size(arm), VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, NULL,
                       &arm->result) ||
        !create_set(arm, buffers, 1) || !create_draw_pipeline(arm) || !create_target(arm) ||
        !create_buffer(dev, sizeof(vertices), VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, vertices,
                       &arm->vertices) ||
        !create_buffer(dev, sizeof(aligned), VK_BUFFER_USAGE_INDEX_BUFFER_BIT,
                       arm->index_offset && !arm->mixed ? unaligned : aligned, &arm->indices) ||
        !create_buffer(dev, sizeof(count), VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT, &count,
                       &arm->count) ||
        !create_buffer(dev, (VkDeviceSize)stride * arm->max_draws,
                       VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT, NULL, &arm->indirect))
        return false;

    /* Every slot holds a draw; the count buffer lets draw_count() of them
     * run. */
    slots = arm->indirect.map;
    for (i = 0; i < arm->max_draws; i++)
    {
        if (indexed)
        {
            VkDrawIndexedIndirectCommand draw = { 3, 1, 0, 0, 0 };
            memcpy(slots + i * stride, &draw, sizeof(draw));
        }
        else
        {
            VkDrawIndirectCommand draw = { 3, 1, 0, 0 };
            memcpy(slots + i * stride, &draw, sizeof(draw));
        }
    }
    return true;
}

static bool setup_loads(struct arm *arm)
{
    const struct buffer *narrow[] = { &arm->sources[0], &arm->weights[0], &arm->result };
    const struct buffer *wide[] = { &arm->sources[0], &arm->sources[1], &arm->sources[2],
                                    &arm->sources[3], &arm->weights[0], &arm->weights[1],
                                    &arm->result };
    uint32_t source_count = arm->wide ? 4 : 1, weight_count = arm->wide ? 2 : 1;
    struct device *dev = arm->dev;
    float *values;
    uint32_t i, j;

    /* All ones, so the sums of each lane are exact (run checks them). */
    for (j = 0; j < source_count; j++)
    {
        if (!create_buffer(dev, SOURCE_VEC4S * 4 * sizeof(float),
                           VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, NULL, &arm->sources[j]))
            return false;
        values = arm->sources[j].map;
        for (i = 0; i < SOURCE_VEC4S * 4; i++) values[i] = 1.0f;
    }
    for (j = 0; j < weight_count; j++)
    {
        if (!create_buffer(dev, 256 * 4 * sizeof(float), VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT, NULL,
                           &arm->weights[j]))
            return false;
        values = arm->weights[j].map;
        for (i = 0; i < 256 * 4; i++) values[i] = 1.0f;
    }
    if (!create_buffer(dev, result_size(arm), VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, NULL,
                       &arm->result))
        return false;
    if (arm->wide)
        return create_set(arm, wide, MAX_BINDINGS) &&
               create_compute_pipeline(arm, loads_wide_comp, sizeof(loads_wide_comp),
                                       2 * sizeof(uint32_t));
    return create_set(arm, narrow, 3) &&
           create_compute_pipeline(arm, loads_comp, sizeof(loads_comp), 2 * sizeof(uint32_t));
}

static bool setup_chain(struct arm *arm)
{
    const struct buffer *buffers[] = { &arm->result };

    return create_buffer(arm->dev, result_size(arm), VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
                         NULL, &arm->result) &&
           create_set(arm, buffers, 1) &&
           create_compute_pipeline(arm, chain_comp, sizeof(chain_comp), 0);
}

static void record_draws(struct arm *arm, VkCommandBuffer cmd, bool check)
{
    VkRenderingAttachmentInfo color = { .sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO,
                                        .imageView = arm->view,
                                        .imageLayout = VK_IMAGE_LAYOUT_GENERAL,
                                        .loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR,
                                        .storeOp = VK_ATTACHMENT_STORE_OP_STORE };
    VkRenderingInfo rendering = { .sType = VK_STRUCTURE_TYPE_RENDERING_INFO,
                                  .renderArea = { { 0, 0 }, { TARGET_SIZE, TARGET_SIZE } },
                                  .layerCount = 1,
                                  .colorAttachmentCount = 1,
                                  .pColorAttachments = &color };
    VkViewport viewport = { 0.0f, 0.0f, TARGET_SIZE, TARGET_SIZE, 0.0f, 1.0f };
    VkRect2D scissor = { { 0, 0 }, { TARGET_SIZE, TARGET_SIZE } };
    VkDeviceSize zero = 0;
    uint32_t i;

    vkCmdBeginRendering(cmd, &rendering);
    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS,
                      check ? arm->check_pipeline : arm->pipeline);
    if (check)
        vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, arm->layout, 0, 1, &arm->set,
                                0, NULL);
    vkCmdSetViewport(cmd, 0, 1, &viewport);
    vkCmdSetScissor(cmd, 0, 1, &scissor);
    vkCmdBindVertexBuffers(cmd, 0, 1, &arm->vertices.buffer, &zero);
    if (arm->kind == KIND_INDEXED)
        vkCmdBindIndexBuffer(cmd, arm->indices.buffer, arm->index_offset, VK_INDEX_TYPE_UINT16);
    for (i = 0; i < CALLS; i++)
    {
        if (arm->mixed)
            vkCmdBindIndexBuffer(cmd, arm->indices.buffer, i % 2 ? arm->index_offset : 0,
                                 VK_INDEX_TYPE_UINT16);
        if (arm->kind == KIND_INDEXED)
            vkCmdDrawIndexedIndirectCount(cmd, arm->indirect.buffer, 0, arm->count.buffer, 0,
                                          arm->max_draws, sizeof(VkDrawIndexedIndirectCommand));
        else
            vkCmdDrawIndirectCount(cmd, arm->indirect.buffer, 0, arm->count.buffer, 0,
                                   arm->max_draws, sizeof(VkDrawIndirectCommand));
    }
    vkCmdEndRendering(cmd);
}

/* Records <count> dispatches of <groups> workgroups of a compute arm: the
 * arm's own work, or a device's load. */
static void record_dispatches(struct arm *arm, VkCommandBuffer cmd, uint32_t count,
                              uint32_t groups)
{
    const uint32_t params[2] = { LOAD_ITERATIONS, SOURCE_VEC4S - 1 };
    VkMemoryBarrier2 memory = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2,
                                .srcStageMask = VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
                                .srcAccessMask = VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT,
                                .dstStageMask = VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
                                .dstAccessMask = VK_ACCESS_2_SHADER_STORAGE_READ_BIT |
                                                 VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT };
    VkDependencyInfo dependency = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                    .memoryBarrierCount = 1,
                                    .pMemoryBarriers = &memory };
    uint32_t i;

    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, arm->pipeline);
    vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, arm->layout, 0, 1, &arm->set, 0,
                            NULL);
    if (arm->kind == KIND_LOADS)
        vkCmdPushConstants(cmd, arm->layout, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof(params),
                           params);
    for (i = 0; i < count; i++)
    {
        vkCmdDispatch(cmd, groups, 1, 1);
        if (arm->barriers && i + 1 < count) vkCmdPipelineBarrier2(cmd, &dependency);
    }
}

/* Runs the arm once: its work between two timestamps, or with <check> its
 * work alone, checked. Stores the GPU time in ms in <ms>. */
static bool run(struct arm *arm, bool check, double *ms)
{
    struct device *dev = arm->dev;
    VkCommandBufferBeginInfo begin = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                       .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VkSubmitInfo submit = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                            .commandBufferCount = 1,
                            .pCommandBuffers = &dev->cmd };
    /* Makes the shader writes the check reads visible to the host. */
    VkMemoryBarrier2 to_host = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2,
                                 .srcStageMask = VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT |
                                                 VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
                                 .srcAccessMask = VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT,
                                 .dstStageMask = VK_PIPELINE_STAGE_2_HOST_BIT,
                                 .dstAccessMask = VK_ACCESS_2_HOST_READ_BIT };
    VkDependencyInfo dependency = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                    .memoryBarrierCount = 1,
                                    .pMemoryBarriers = &to_host };
    uint64_t stamps[2];

    if (check) memset(arm->result.map, 0, result_size(arm));
    vkResetQueryPool(dev->device, dev->timestamps, 0, 2);
    if (!ok(vkResetFences(dev->device, 1, &dev->fence), "vkResetFences") ||
        !ok(vkResetCommandBuffer(dev->cmd, 0), "vkResetCommandBuffer") ||
        !ok(vkBeginCommandBuffer(dev->cmd, &begin), "vkBeginCommandBuffer"))
        return false;
    if (!check)
    {
        /* The first timestamp waits for the work before it, so the load is
         * outside the interval. */
        record_dispatches(&loads[arm->robustness], dev->cmd, LOAD_DISPATCHES, LOAD_GROUPS);
        vkCmdWriteTimestamp(dev->cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, dev->timestamps, 0);
    }
    if (arm->kind == KIND_INDEXED || arm->kind == KIND_NONINDEXED)
        record_draws(arm, dev->cmd, check);
    else
        record_dispatches(arm, dev->cmd, arm->kind == KIND_LOADS ? DISPATCHES : CHAIN_LENGTH, 1);
    if (check)
        vkCmdPipelineBarrier2(dev->cmd, &dependency);
    else
        vkCmdWriteTimestamp(dev->cmd, VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, dev->timestamps, 1);
    if (!ok(vkEndCommandBuffer(dev->cmd), "vkEndCommandBuffer") ||
        !ok(vkQueueSubmit(dev->queue, 1, &submit, dev->fence), "vkQueueSubmit") ||
        !ok(vkWaitForFences(dev->device, 1, &dev->fence, VK_TRUE, UINT64_MAX),
            "vkWaitForFences"))
        return false;

    if (check)
    {
        char what[160];
        settings(arm, what, sizeof(what));
        if (arm->kind == KIND_INDEXED || arm->kind == KIND_NONINDEXED)
        {
            uint32_t fragments = *(const uint32_t *)arm->result.map;
            uint32_t want = CALLS * draw_count(arm) * TARGET_SIZE * TARGET_SIZE;
            if (fragments != want)
            {
                printf("kk: FAIL %s %s: the draws produced %u fragments, not %u\n", arm->name,
                       what, fragments, want);
                return false;
            }
        }
        else
        {
            uint32_t i;
            for (i = 0; i < 32; i++)
            {
                bool good;
                if (arm->kind == KIND_LOADS)
                {
                    /* Each iteration adds 1 per term: one in loads.comp, four
                     * in loads_wide.comp. */
                    const float want = (arm->wide ? 4.0f : 1.0f) * LOAD_ITERATIONS;
                    const float *sum = (const float *)arm->result.map + 4 * i;
                    good = sum[0] == want && sum[1] == want && sum[2] == want && sum[3] == want;
                }
                else
                {
                    /* Without barriers the dispatches race on the counters. */
                    good = !arm->barriers || ((const uint32_t *)arm->result.map)[i] == CHAIN_LENGTH;
                }
                if (!good)
                {
                    printf("kk: FAIL %s %s: lane %u computed a wrong result\n", arm->name, what, i);
                    return false;
                }
            }
        }
        return true;
    }

    if (!ok(vkGetQueryPoolResults(dev->device, dev->timestamps, 0, 2, sizeof(stamps), stamps,
                                  sizeof(stamps[0]),
                                  VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WAIT_BIT),
            "vkGetQueryPoolResults"))
        return false;
    if (stamps[1] == stamps[0])
    {
        /* Mesa before "kk: Write compute timestamps with precise granularity"
         * (#115) gives the timestamps of one compute encoder one value. */
        printf("kk: FAIL %s: both timestamps read %llu, so this KosmicKrisp cannot time it\n",
               arm->name, (unsigned long long)stamps[0]);
        return false;
    }
    *ms = (double)((stamps[1] - stamps[0]) & timestamp_mask) * timestamp_period / 1e6;
    return true;
}

static bool setup(struct arm *arm)
{
    struct device *dev = &devices[arm->robustness];
    double unused;

    if (!dev->created)
    {
        struct arm *load = &loads[arm->robustness];

        load->dev = dev;
        if (create_device(dev, arm->robustness) && !setup_loads(load)) dev->failed = true;
    }
    if (dev->failed) return false;
    arm->dev = dev;
    switch (arm->kind)
    {
    case KIND_INDEXED:
    case KIND_NONINDEXED:
        if (!setup_draw(arm)) return false;
        break;
    case KIND_LOADS:
        if (!setup_loads(arm)) return false;
        break;
    case KIND_CHAIN:
        if (!setup_chain(arm)) return false;
        break;
    }
    return run(arm, true, &unused);
}

static void teardown(struct arm *arm)
{
    struct device *dev = arm->dev;
    uint32_t i;

    if (!dev || !dev->device) return;
    if (arm->pipeline) vkDestroyPipeline(dev->device, arm->pipeline, NULL);
    if (arm->check_pipeline) vkDestroyPipeline(dev->device, arm->check_pipeline, NULL);
    if (arm->layout) vkDestroyPipelineLayout(dev->device, arm->layout, NULL);
    if (arm->set_layout) vkDestroyDescriptorSetLayout(dev->device, arm->set_layout, NULL);
    if (arm->view) vkDestroyImageView(dev->device, arm->view, NULL);
    if (arm->image) vkDestroyImage(dev->device, arm->image, NULL);
    if (arm->image_memory) vkFreeMemory(dev->device, arm->image_memory, NULL);
    destroy_buffer(dev, &arm->vertices);
    destroy_buffer(dev, &arm->indices);
    destroy_buffer(dev, &arm->indirect);
    destroy_buffer(dev, &arm->count);
    for (i = 0; i < 4; i++) destroy_buffer(dev, &arm->sources[i]);
    for (i = 0; i < 2; i++) destroy_buffer(dev, &arm->weights[i]);
    destroy_buffer(dev, &arm->result);
    free(arm->samples);
}

static void destroy_device(struct device *dev)
{
    if (!dev->device) return;
    vkDeviceWaitIdle(dev->device);
    if (dev->descriptors) vkDestroyDescriptorPool(dev->device, dev->descriptors, NULL);
    if (dev->timestamps) vkDestroyQueryPool(dev->device, dev->timestamps, NULL);
    if (dev->fence) vkDestroyFence(dev->device, dev->fence, NULL);
    if (dev->pool) vkDestroyCommandPool(dev->device, dev->pool, NULL);
    vkDestroyDevice(dev->device, NULL);
}

static int compare(const void *a, const void *b)
{
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}

/* The <p> quantile of the <n> sorted <samples>, interpolated between the two
 * nearest. */
static double quantile(const double *samples, unsigned n, double p)
{
    double pos = p * (n - 1);
    unsigned below = (unsigned)pos;

    if (below + 1 >= n) return samples[n - 1];
    return samples[below] + (pos - below) * (samples[below + 1] - samples[below]);
}

/* The spread is the quartiles: a single round slowed by something else on
 * the GPU moves the maximum by more than the differences the cases show. */
static void report(struct arm *arm, unsigned rounds)
{
    const double *s = arm->samples;
    char what[160];

    qsort(arm->samples, rounds, sizeof(double), compare);
    settings(arm, what, sizeof(what));
    printf("%s %s: median %.3f ms, quartiles %.3f to %.3f, min %.3f, max %.3f, n=%u\n",
           arm->name, what, quantile(s, rounds, 0.5), quantile(s, rounds, 0.25),
           quantile(s, rounds, 0.75), s[0], s[rounds - 1], rounds);
}

/* Picks the first device and a queue family with graphics, compute and
 * timestamps, and prints what the stack is. */
static bool pick_device(void)
{
    VkPhysicalDeviceDriverProperties driver = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES,
    };
    VkPhysicalDeviceProperties2 props = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2,
                                          .pNext = &driver };
    VkQueueFamilyProperties families[16];
    uint32_t count = 1, i;
    VkResult res;

    res = vkEnumeratePhysicalDevices(instance, &count, &physical);
    if ((res != VK_SUCCESS && res != VK_INCOMPLETE) || !count)
    {
        printf("kk: no Vulkan device, result %d\n", (int)res);
        return false;
    }
    vkGetPhysicalDeviceProperties2(physical, &props);
    vkGetPhysicalDeviceMemoryProperties(physical, &memory_properties);
    timestamp_period = props.properties.limits.timestampPeriod;

    count = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(physical, &count, families);
    for (i = 0; i < count; i++)
        if ((families[i].queueFlags & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT)) ==
                (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT) &&
            families[i].timestampValidBits)
            break;
    if (i == count)
    {
        printf("kk: no queue family with graphics, compute and timestamps\n");
        return false;
    }
    queue_family = i;
    timestamp_mask = families[i].timestampValidBits >= 64
                         ? UINT64_MAX
                         : (1ull << families[i].timestampValidBits) - 1;

    printf("kk: device %s, driver %s (%s), driverVersion %#x\n", props.properties.deviceName,
           driver.driverName, driver.driverInfo, props.properties.driverVersion);
    printf("kk: timestampPeriod %g ns\n", timestamp_period);
    for (i = 0; i < _dyld_image_count(); i++)
    {
        const char *name = _dyld_get_image_name(i);
        if (strstr(name, "libvulkan")) printf("kk: loaded %s\n", name);
    }
    return true;
}

static void usage(const char *argv0)
{
    size_t i;

    printf("usage: %s <libvulkan.1.dylib> [--rounds <n>] [--case <name>]...\n", argv0);
    printf("cases:");
    for (i = 0; i < ARM_COUNT; i++)
        if (!i || strcmp(arms[i].name, arms[i - 1].name)) printf(" %s", arms[i].name);
    printf("\n");
}

int main(int argc, char **argv)
{
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .pApplicationName = "mach9 kk bench",
                              .apiVersion = VK_API_VERSION_1_3 };
    VkInstanceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                  .pApplicationInfo = &app };
    unsigned rounds = DEFAULT_ROUNDS, round;
    bool any_case = false, failed = false;
    struct arm *order[ARM_COUNT];
    size_t i, j, selected = 0;
    void *loader;
    int arg;

    if (argc < 2)
    {
        usage(argv[0]);
        return 2;
    }
    for (arg = 2; arg < argc; arg++)
    {
        if (!strcmp(argv[arg], "--rounds") && arg + 1 < argc)
        {
            char *end;
            unsigned long value = strtoul(argv[++arg], &end, 10);
            if (*end || !value || value > 100000)
            {
                usage(argv[0]);
                return 2;
            }
            rounds = (unsigned)value;
        }
        else if (!strcmp(argv[arg], "--case") && arg + 1 < argc)
        {
            bool known = false;
            arg++;
            for (i = 0; i < ARM_COUNT; i++)
                if (!strcmp(arms[i].name, argv[arg])) arms[i].selected = known = true;
            if (!known)
            {
                usage(argv[0]);
                return 2;
            }
            any_case = true;
        }
        else
        {
            usage(argv[0]);
            return 2;
        }
    }
    if (!any_case)
        for (i = 0; i < ARM_COUNT; i++) arms[i].selected = true;

    if (!(loader = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("kk: cannot load %s: %s\n", argv[1], dlerror());
        return 1;
    }
    get_instance_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    if (!get_instance_proc)
    {
        printf("kk: %s has no vkGetInstanceProcAddr\n", argv[1]);
        return 1;
    }
    create_instance = (PFN_vkCreateInstance)get_instance_proc(NULL, "vkCreateInstance");
    if (!create_instance || !ok(create_instance(&info, NULL, &instance), "vkCreateInstance"))
        return 1;
#define LOAD(name)                                                                                 \
    if (!(name = (PFN_##name)get_instance_proc(instance, #name)))                                  \
    {                                                                                              \
        printf("kk: the loader has no %s\n", #name);                                               \
        return 1;                                                                                  \
    }
    INSTANCE_FUNCS(LOAD)
#undef LOAD
    if (!pick_device()) return 1;

    for (i = 0; i < ARM_COUNT; i++)
    {
        struct arm *arm = &arms[i];
        if (!arm->selected) continue;
        if (!setup(arm))
        {
            char what[160];
            settings(arm, what, sizeof(what));
            printf("%s %s: FAIL, not timed\n", arm->name, what);
            arm->failed = failed = true;
            continue;
        }
        if (!(arm->samples = calloc(rounds, sizeof(double))))
        {
            printf("kk: out of memory\n");
            return 1;
        }
        arm->ready = true;
        order[selected++] = arm;
    }

    /* Round r starts with the (r mod n)th arm and keeps the order, so each
     * round has another first arm. */
    for (round = 0; round < WARMUP_ROUNDS + rounds; round++)
    {
        for (j = 0; j < selected; j++)
        {
            struct arm *arm = order[(round + j) % selected];
            double ms;
            if (!run(arm, false, &ms))
            {
                printf("kk: %s stopped in round %u\n", arm->name, round);
                return 1;
            }
            if (round >= WARMUP_ROUNDS) arm->samples[round - WARMUP_ROUNDS] = ms;
        }
    }

    for (i = 0; i < ARM_COUNT; i++)
        if (arms[i].ready) report(&arms[i], rounds);

    for (i = 0; i < ARM_COUNT; i++) teardown(&arms[i]);
    for (i = 0; i < ROBUST_COUNT; i++)
    {
        teardown(&loads[i]);
        destroy_device(&devices[i]);
    }
    vkDestroyInstance(instance, NULL);
    return failed;
}
