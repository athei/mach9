/* GPU time of KosmicKrisp on the patterns behind the Enshrouded gap (#36):
 * indirect-count draws with a large maxDrawCount (#61), buffer loads under
 * robust buffer access (#62) and chains of compute barriers (#63); of
 * timestamps written outside render passes (#117); and
 * whether fragment shaders with 64-bit atomics lose the device. It runs
 * natively, outside Wine, on the Vulkan loader it is given, which takes the
 * driver from VK_DRIVER_FILES. Build and run it with kk.sh next to it:
 *
 *     bench/kk.sh [<bundle or MESA_DIST>] [--rounds <n>] [--case <name>]...
 *
 * The stack defaults to dist/wine. A MESA_DIST of bin/build-mesa.sh compares
 * two Mesa builds without a bundle step. --case runs only the named cases;
 * the default runs all timed cases.
 *
 * The atomic64 cases are not timed and run only when named. Each draws for
 * STRESS_SECONDS with 64-bit atomics in the fragment shader, which
 * KosmicKrisp runs under spin locks from a table of lock words, one submit
 * per draw, on a device of its own, and prints whether the device was lost
 * (a lock that times out loses it) and whether the counters hold what the
 * draws added (#153):
 *
 * | Case | Draw | Each fragment |
 * | --- | --- | --- |
 * | atomic64-exchange | 248x248 | exchanges its own 64-bit guard in a storage buffer, then its neighbour's, the pattern of #91 |
 * | atomic64-one-texel | 2560x1440 | adds 1 to texel (0, 0) of an R64_UINT storage image: every fragment on one lock |
 * | atomic64-own-texel | 2560x1440 | adds 1 to its own texel of an R64_UINT storage image of the draw's size |
 *
 * A draw that does not finish within STRESS_FENCE_TIMEOUT_NS ends the
 * program. A lost device can make other processes' work on the GPU fail too,
 * so run these cases with nothing else on the GPU.
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
 * the exit status is 1. A draw arm's check counts its fragments twice: with
 * an atomic in the fragment shader of a pipeline of its own, and with a
 * precise occlusion query around all its calls, reset by vkCmdResetQueryPool
 * in the same command buffer. Every indirect-count call ends KosmicKrisp's
 * Metal render encoder, so the query fails on a KosmicKrisp whose encoders
 * reset the visibility results, as before "kk: Accumulate occlusion results
 * across the render encoders of a pass" (#120). A wrong fragment count
 * keeps the arm from being timed; a wrong query count only prints FAIL and
 * sets the exit status to 1, so the draw cases still time such a build.
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
 * | timestamps | TIMESTAMP_COUNT timestamps, 1 timestamp | vkCmdWriteTimestamp into each query of a pool of that size, in order, then one vkCmdCopyQueryPoolResults of them all |
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
 * - The timestamps case times what timestamps cost, resolves included: the
 *   copy waits for the resolves of the timestamps before it, so they fall
 *   inside the interval. The difference between its arms over
 *   TIMESTAMP_COUNT - 1 is the GPU time of one timestamp. Each round uses
 *   the counter heap entries the round before gave back, so the arms show
 *   whether a KosmicKrisp resolves the timestamps of consecutive queries in
 *   one call after a reset. The check compares the copied values with
 *   vkGetQueryPoolResults and requires them non-zero and not decreasing.
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
#include <time.h>

#include "shaders/atomic64_exchange.frag.h"
#include "shaders/atomic64_texel.frag.h"
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
/* The timestamps of the larger timestamps arm, those of the Metal
 * measurement in #117. */
#define TIMESTAMP_COUNT 1024
/* The bindings of loads_wide.comp, the most of any shader here. */
#define MAX_BINDINGS 7
/* The load before each timed interval: about 29 ms of GPU time on an M5 Max,
 * against the 16 ms of work the GPU took from idle to its highest
 * performance state. */
#define LOAD_DISPATCHES 256
#define LOAD_GROUPS 1024
#define WARMUP_ROUNDS 3
#define DEFAULT_ROUNDS 20
/* The atomic64 cases: the draw of #91's 248x248 pattern, the draw of the
 * image cases, how long each case draws, and how long one draw may take
 * before it counts as a stall. */
#define EXCHANGE_SIZE 248
#define STRESS_WIDTH 2560
#define STRESS_HEIGHT 1440
#define STRESS_SECONDS 30
#define STRESS_FENCE_TIMEOUT_NS 60000000000ull

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
    X(vkCmdResetQueryPool)                                                                         \
    X(vkCmdBeginQuery)                                                                             \
    X(vkCmdEndQuery)                                                                               \
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
    X(vkCmdDrawIndexedIndirectCount)                                                               \
    X(vkCmdDraw)                                                                                   \
    X(vkCmdCopyBufferToImage)                                                                      \
    X(vkCmdCopyImageToBuffer)                                                                      \
    X(vkCmdCopyQueryPoolResults)

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
    KIND_TIMESTAMPS,
    KIND_ATOMIC64,
};

/* What a fragment of an atomic64 case does. */
enum pattern
{
    PATTERN_EXCHANGE,
    PATTERN_ONE_TEXEL,
    PATTERN_OWN_TEXEL,
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
    /* The precise occlusion query of the draw arms' checks. */
    VkQueryPool occlusion;
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
    enum pattern pattern;
    /* timestamps: the timestamps written between the two of the interval. */
    uint32_t timestamps;

    bool selected, ready, failed;
    /* A draw arm's check: the occlusion query missed the expected count. */
    bool query_failed;
    struct device *dev;
    VkPipelineLayout layout;
    VkDescriptorSetLayout set_layout;
    VkDescriptorSet set;
    VkPipeline pipeline;
    VkPipeline check_pipeline;
    VkImage image;
    VkDeviceMemory image_memory;
    VkImageView view;
    /* The R64_UINT storage image of the atomic64 image cases. */
    VkImage counters;
    VkDeviceMemory counters_memory;
    VkImageView counters_view;
    /* The query pool of a timestamps arm. */
    VkQueryPool queries;
    /* result: the fragment counter of a draw arm's check, what a compute
     * arm writes, or the guards of atomic64-exchange. readback: the
     * counters of the atomic64 image cases, zero before the first draw. */
    struct buffer vertices, indices, indirect, count, sources[4], weights[2], result, readback;
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
#define TIMESTAMPS(n) { .name = "timestamps", .kind = KIND_TIMESTAMPS, .timestamps = n }
#define ATOMIC64(case_name, what) { .name = case_name, .kind = KIND_ATOMIC64, .pattern = what }

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
    TIMESTAMPS(TIMESTAMP_COUNT),
    TIMESTAMPS(1),
    ATOMIC64("atomic64-exchange", PATTERN_EXCHANGE),
    ATOMIC64("atomic64-one-texel", PATTERN_ONE_TEXEL),
    ATOMIC64("atomic64-own-texel", PATTERN_OWN_TEXEL),
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
/* The device of the atomic64 case that runs, one per case, since a case can
 * lose it. */
static struct device atomic64_device;

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

/* The size of an atomic64 case's draw. */
static uint32_t atomic64_width(const struct arm *arm)
{
    return arm->pattern == PATTERN_EXCHANGE ? EXCHANGE_SIZE : STRESS_WIDTH;
}

static uint32_t atomic64_height(const struct arm *arm)
{
    return arm->pattern == PATTERN_EXCHANGE ? EXCHANGE_SIZE : STRESS_HEIGHT;
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
    case KIND_TIMESTAMPS:
        snprintf(out, size, "timestamps=%u", arm->timestamps);
        break;
    case KIND_ATOMIC64:
        snprintf(out, size, "draw=%ux%u seconds=%u", atomic64_width(arm), atomic64_height(arm),
                 STRESS_SECONDS);
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

/* Creates the device of <robustness> with every feature the timed cases use,
 * or with <atomic64> the device of an atomic64 case, which also has 64-bit
 * buffer and image atomics. */
static bool create_device(struct device *dev, enum robustness robustness, bool atomic64)
{
    static const float priority = 1.0f;
    VkPhysicalDeviceShaderImageAtomicInt64FeaturesEXT image_int64 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_IMAGE_ATOMIC_INT64_FEATURES_EXT,
        .shaderImageInt64Atomics = VK_TRUE,
    };
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
        .features.occlusionQueryPrecise = VK_TRUE,
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
    VkQueryPoolCreateInfo occlusion = { .sType = VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO,
                                        .queryType = VK_QUERY_TYPE_OCCLUSION,
                                        .queryCount = 1 };
    VkDescriptorPoolSize sizes[] = {
        { VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, MAX_BINDINGS * (ARM_COUNT + 1) },
        { VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, MAX_BINDINGS * (ARM_COUNT + 1) },
        { VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1 },
    };
    /* The arms and the device's load. */
    VkDescriptorPoolCreateInfo descriptors = { .sType =
                                                   VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                                               .maxSets = ARM_COUNT + 1,
                                               .poolSizeCount = atomic64 ? 3 : 2,
                                               .pPoolSizes = sizes };

    if (atomic64)
    {
        features.features.shaderInt64 = VK_TRUE;
        features12.shaderBufferInt64Atomics = VK_TRUE;
        features13.pNext = &image_int64;
        extension = VK_EXT_SHADER_IMAGE_ATOMIC_INT64_EXTENSION_NAME;
    }
    else if (robustness == ROBUST_RBA2)
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
        !ok(vkCreateQueryPool(dev->device, &occlusion, NULL, &dev->occlusion),
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
    VkShaderStageFlags stage = arm->kind == KIND_INDEXED || arm->kind == KIND_NONINDEXED ||
                                       arm->kind == KIND_ATOMIC64
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
    /* The draw's width, or whether it adds to one texel, for an atomic64
     * case. */
    VkPushConstantRange push = { VK_SHADER_STAGE_FRAGMENT_BIT, 0, sizeof(uint32_t) };
    VkPipelineLayoutCreateInfo layout = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                                          .setLayoutCount = 1,
                                          .pSetLayouts = &arm->set_layout,
                                          .pushConstantRangeCount =
                                              arm->kind == KIND_ATOMIC64 ? 1 : 0,
                                          .pPushConstantRanges = &push };
    const bool exchange = arm->kind == KIND_ATOMIC64 && arm->pattern == PATTERN_EXCHANGE;
    bool created = false;

    if (!ok(vkCreatePipelineLayout(arm->dev->device, &layout, NULL, &arm->layout),
            "vkCreatePipelineLayout"))
        return false;
    info.layout = arm->layout;
    stages[0].module = create_module(arm->dev, draw_vert, sizeof(draw_vert));
    if (arm->kind != KIND_ATOMIC64)
        stages[1].module = create_module(arm->dev, draw_frag, sizeof(draw_frag));
    else if (exchange)
        stages[1].module =
            create_module(arm->dev, atomic64_exchange_frag, sizeof(atomic64_exchange_frag));
    else
        stages[1].module = create_module(arm->dev, atomic64_texel_frag, sizeof(atomic64_texel_frag));
    if (stages[0].module && stages[1].module)
        created = ok(vkCreateGraphicsPipelines(arm->dev->device, VK_NULL_HANDLE, 1, &info, NULL,
                                               &arm->pipeline),
                     "vkCreateGraphicsPipelines");
    if (stages[1].module) vkDestroyShaderModule(arm->dev->device, stages[1].module, NULL);
    if (arm->kind == KIND_ATOMIC64)
    {
        if (stages[0].module) vkDestroyShaderModule(arm->dev->device, stages[0].module, NULL);
        return created;
    }
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

/* A <width> x <height> image of <format> for <usage> with its view, left in
 * VK_IMAGE_LAYOUT_GENERAL: the render target of a draw arm or of an atomic64
 * case, or the storage image of an atomic64 image case. */
static bool create_target(struct arm *arm, VkFormat format, VkImageUsageFlags usage,
                          uint32_t width, uint32_t height, VkImage *out,
                          VkDeviceMemory *memory, VkImageView *view_out)
{
    struct device *dev = arm->dev;
    VkImageCreateInfo image = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                                .imageType = VK_IMAGE_TYPE_2D,
                                .format = format,
                                .extent = { width, height, 1 },
                                .mipLevels = 1,
                                .arrayLayers = 1,
                                .samples = VK_SAMPLE_COUNT_1_BIT,
                                .tiling = VK_IMAGE_TILING_OPTIMAL,
                                .usage = usage,
                                .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
                                .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    VkImageViewCreateInfo view = { .sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                                   .viewType = VK_IMAGE_VIEW_TYPE_2D,
                                   .format = format,
                                   .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };
    VkImageMemoryBarrier2 barrier = { .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
                                      .dstStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
                                      .dstAccessMask = VK_ACCESS_2_MEMORY_READ_BIT |
                                                       VK_ACCESS_2_MEMORY_WRITE_BIT,
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

    if (!ok(vkCreateImage(dev->device, &image, NULL, out), "vkCreateImage")) return false;
    vkGetImageMemoryRequirements(dev->device, *out, &req);
    if ((type = find_memory_type(req.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)) < 0)
        type = find_memory_type(req.memoryTypeBits, 0);
    alloc.allocationSize = req.size;
    alloc.memoryTypeIndex = (uint32_t)type;
    if (type < 0 ||
        !ok(vkAllocateMemory(dev->device, &alloc, NULL, memory), "vkAllocateMemory") ||
        !ok(vkBindImageMemory(dev->device, *out, *memory, 0), "vkBindImageMemory"))
        return false;
    view.image = *out;
    if (!ok(vkCreateImageView(dev->device, &view, NULL, view_out), "vkCreateImageView"))
        return false;

    barrier.image = *out;
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
 * compute arm, the copied timestamps of a timestamps arm, the fragment
 * counter of a draw arm. */
static size_t result_size(const struct arm *arm)
{
    switch (arm->kind)
    {
    case KIND_LOADS:
        return 32 * 4 * sizeof(float);
    case KIND_CHAIN:
        return 32 * sizeof(uint32_t);
    case KIND_TIMESTAMPS:
        return arm->timestamps * sizeof(uint64_t);
    default:
        return sizeof(uint32_t);
    }
}

/* A triangle that covers the whole target, so each draw of a draw arm's
 * check counts TARGET_SIZE * TARGET_SIZE fragments, and each draw of an
 * atomic64 case one fragment per pixel. */
static const float vertices[] = { -1.0f, -1.0f, 3.0f, -1.0f, -1.0f, 3.0f };

static bool setup_draw(struct arm *arm)
{
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
        !create_set(arm, buffers, 1) || !create_draw_pipeline(arm) ||
        !create_target(arm, VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
                       TARGET_SIZE, TARGET_SIZE, &arm->image, &arm->image_memory, &arm->view) ||
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

static bool setup_timestamps(struct arm *arm)
{
    VkQueryPoolCreateInfo info = { .sType = VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO,
                                   .queryType = VK_QUERY_TYPE_TIMESTAMP,
                                   .queryCount = arm->timestamps };

    return create_buffer(arm->dev, result_size(arm), VK_BUFFER_USAGE_TRANSFER_DST_BIT, NULL,
                         &arm->result) &&
           ok(vkCreateQueryPool(arm->dev->device, &info, NULL, &arm->queries),
              "vkCreateQueryPool");
}

/* Records the work of a timestamps arm: a timestamp into each query of its
 * pool, in order, and one copy of them all into its result buffer, which
 * waits for their resolves. */
static void record_timestamps(struct arm *arm, VkCommandBuffer cmd)
{
    uint32_t i;

    for (i = 0; i < arm->timestamps; i++)
        vkCmdWriteTimestamp(cmd, VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, arm->queries, i);
    vkCmdCopyQueryPoolResults(cmd, arm->queries, 0, arm->timestamps, arm->result.buffer, 0,
                              sizeof(uint64_t),
                              VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WAIT_BIT);
}

/* The check of a timestamps arm: every copied timestamp is non-zero, none
 * is earlier than the one before, the last is later than the first, and
 * each equals what vkGetQueryPoolResults reads. */
static bool check_timestamps(struct arm *arm, char *wrong, size_t size)
{
    const uint64_t *copied = arm->result.map;
    uint64_t *read = calloc(arm->timestamps, sizeof(uint64_t));
    bool good = false;
    uint32_t i;

    if (!read)
    {
        snprintf(wrong, size, "out of memory");
        return false;
    }
    if (!ok(vkGetQueryPoolResults(arm->dev->device, arm->queries, 0, arm->timestamps,
                                  arm->timestamps * sizeof(uint64_t), read, sizeof(uint64_t),
                                  VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WAIT_BIT),
            "vkGetQueryPoolResults"))
    {
        snprintf(wrong, size, "the results could not be read");
        goto out;
    }
    for (i = 0; i < arm->timestamps; i++)
    {
        if (!copied[i] || copied[i] != read[i] || (i && copied[i] < copied[i - 1]))
        {
            snprintf(wrong, size, "timestamp %u was copied as %llu and reads %llu, after %llu", i,
                     (unsigned long long)copied[i], (unsigned long long)read[i],
                     i ? (unsigned long long)copied[i - 1] : 0ull);
            goto out;
        }
    }
    if (arm->timestamps > 1 && copied[arm->timestamps - 1] == copied[0])
    {
        snprintf(wrong, size, "all %u timestamps read %llu", arm->timestamps,
                 (unsigned long long)copied[0]);
        goto out;
    }
    good = true;

out:
    free(read);
    return good;
}

/* The descriptor set of an atomic64 image case: its storage image at
 * binding 0. */
static bool create_image_set(struct arm *arm)
{
    VkDescriptorSetLayoutBinding binding = { .binding = 0,
                                             .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
                                             .descriptorCount = 1,
                                             .stageFlags = VK_SHADER_STAGE_FRAGMENT_BIT };
    VkDescriptorSetLayoutCreateInfo layout = { .sType =
                                                   VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
                                               .bindingCount = 1,
                                               .pBindings = &binding };
    VkDescriptorSetAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                                          .descriptorPool = arm->dev->descriptors,
                                          .descriptorSetCount = 1,
                                          .pSetLayouts = &arm->set_layout };
    VkDescriptorImageInfo info = { .imageView = arm->counters_view,
                                   .imageLayout = VK_IMAGE_LAYOUT_GENERAL };
    VkWriteDescriptorSet write = { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                                   .dstBinding = 0,
                                   .descriptorCount = 1,
                                   .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
                                   .pImageInfo = &info };

    if (!ok(vkCreateDescriptorSetLayout(arm->dev->device, &layout, NULL, &arm->set_layout),
            "vkCreateDescriptorSetLayout") ||
        !ok(vkAllocateDescriptorSets(arm->dev->device, &alloc, &arm->set),
            "vkAllocateDescriptorSets"))
        return false;
    write.dstSet = arm->set;
    vkUpdateDescriptorSets(arm->dev->device, 1, &write, 0, NULL);
    return true;
}

/* The guards of atomic64-exchange, or the storage image of the image cases
 * with a readback buffer as large as it, which also clears it; then the
 * pipeline and the render target. */
static bool setup_atomic64(struct arm *arm)
{
    const uint32_t width = atomic64_width(arm), height = atomic64_height(arm);
    const struct buffer *buffers[] = { &arm->result };
    struct device *dev = arm->dev;

    if (arm->pattern == PATTERN_EXCHANGE)
    {
        if (!create_buffer(dev, (VkDeviceSize)width * height * sizeof(uint64_t),
                           VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, NULL, &arm->result) ||
            !create_set(arm, buffers, 1))
            return false;
    }
    else
    {
        /* One texel is enough for atomic64-one-texel. */
        const uint32_t texels_x = arm->pattern == PATTERN_ONE_TEXEL ? 1 : width;
        const uint32_t texels_y = arm->pattern == PATTERN_ONE_TEXEL ? 1 : height;

        if (!create_target(arm, VK_FORMAT_R64_UINT,
                           VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                               VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                           texels_x, texels_y, &arm->counters, &arm->counters_memory,
                           &arm->counters_view) ||
            !create_buffer(dev, (VkDeviceSize)texels_x * texels_y * sizeof(uint64_t),
                           VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT,
                           NULL, &arm->readback) ||
            !create_image_set(arm))
            return false;
    }
    return create_draw_pipeline(arm) &&
           create_target(arm, VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT, width,
                         height, &arm->image, &arm->image_memory, &arm->view) &&
           create_buffer(dev, sizeof(vertices), VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, vertices,
                         &arm->vertices);
}

/* Copies the storage image of an atomic64 image case to or from its
 * readback buffer, as the last or the first command of dev->cmd. */
static void copy_counters(struct arm *arm, bool to_image)
{
    const bool one = arm->pattern == PATTERN_ONE_TEXEL;
    VkBufferImageCopy region = {
        .imageSubresource = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1 },
        .imageExtent = { one ? 1 : atomic64_width(arm), one ? 1 : atomic64_height(arm), 1 },
    };
    VkMemoryBarrier2 memory = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2,
                                .srcStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
                                .srcAccessMask = VK_ACCESS_2_MEMORY_WRITE_BIT,
                                .dstStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
                                .dstAccessMask = VK_ACCESS_2_MEMORY_READ_BIT |
                                                 VK_ACCESS_2_MEMORY_WRITE_BIT };
    VkDependencyInfo dependency = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                    .memoryBarrierCount = 1,
                                    .pMemoryBarriers = &memory };

    vkCmdPipelineBarrier2(arm->dev->cmd, &dependency);
    if (to_image)
        vkCmdCopyBufferToImage(arm->dev->cmd, arm->readback.buffer, arm->counters,
                               VK_IMAGE_LAYOUT_GENERAL, 1, &region);
    else
        vkCmdCopyImageToBuffer(arm->dev->cmd, arm->counters, VK_IMAGE_LAYOUT_GENERAL,
                               arm->readback.buffer, 1, &region);
    vkCmdPipelineBarrier2(arm->dev->cmd, &dependency);
}

/* Records one draw of an atomic64 case, after the clear of its counters
 * with <first>, or only the copy of its counters to the readback buffer with
 * <readback>; submits it and waits for it. Returns what the submit or the
 * wait returned. */
static VkResult run_atomic64(struct arm *arm, bool first, bool readback)
{
    const uint32_t width = atomic64_width(arm), height = atomic64_height(arm);
    struct device *dev = arm->dev;
    VkCommandBufferBeginInfo begin = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                       .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VkSubmitInfo submit = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                            .commandBufferCount = 1,
                            .pCommandBuffers = &dev->cmd };
    VkRenderingAttachmentInfo color = { .sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO,
                                        .imageView = arm->view,
                                        .imageLayout = VK_IMAGE_LAYOUT_GENERAL,
                                        .loadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE,
                                        .storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE };
    VkRenderingInfo rendering = { .sType = VK_STRUCTURE_TYPE_RENDERING_INFO,
                                  .renderArea = { { 0, 0 }, { width, height } },
                                  .layerCount = 1,
                                  .colorAttachmentCount = 1,
                                  .pColorAttachments = &color };
    VkViewport viewport = { 0.0f, 0.0f, (float)width, (float)height, 0.0f, 1.0f };
    VkRect2D scissor = { { 0, 0 }, { width, height } };
    /* Makes the atomics of one draw visible to the next one and to the host. */
    VkMemoryBarrier2 after = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2,
                               .srcStageMask = VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT,
                               .srcAccessMask = VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT,
                               .dstStageMask = VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT |
                                               VK_PIPELINE_STAGE_2_HOST_BIT,
                               .dstAccessMask = VK_ACCESS_2_SHADER_STORAGE_READ_BIT |
                                                VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT |
                                                VK_ACCESS_2_HOST_READ_BIT };
    VkDependencyInfo dependency = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                    .memoryBarrierCount = 1,
                                    .pMemoryBarriers = &after };
    uint32_t push = arm->pattern == PATTERN_EXCHANGE ? width : arm->pattern == PATTERN_ONE_TEXEL;
    VkDeviceSize zero = 0;
    VkResult result;

    if ((result = vkResetFences(dev->device, 1, &dev->fence)) ||
        (result = vkResetCommandBuffer(dev->cmd, 0)) ||
        (result = vkBeginCommandBuffer(dev->cmd, &begin)))
        return result;
    if (readback)
        copy_counters(arm, false);
    else
    {
        if (first && arm->pattern != PATTERN_EXCHANGE) copy_counters(arm, true);
        vkCmdBeginRendering(dev->cmd, &rendering);
        vkCmdBindPipeline(dev->cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, arm->pipeline);
        vkCmdBindDescriptorSets(dev->cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, arm->layout, 0, 1,
                                &arm->set, 0, NULL);
        vkCmdPushConstants(dev->cmd, arm->layout, VK_SHADER_STAGE_FRAGMENT_BIT, 0, sizeof(push),
                           &push);
        vkCmdSetViewport(dev->cmd, 0, 1, &viewport);
        vkCmdSetScissor(dev->cmd, 0, 1, &scissor);
        vkCmdBindVertexBuffers(dev->cmd, 0, 1, &arm->vertices.buffer, &zero);
        vkCmdDraw(dev->cmd, 3, 1, 0, 0);
        vkCmdEndRendering(dev->cmd);
        vkCmdPipelineBarrier2(dev->cmd, &dependency);
    }
    if ((result = vkEndCommandBuffer(dev->cmd)) ||
        (result = vkQueueSubmit(dev->queue, 1, &submit, dev->fence)))
        return result;
    return vkWaitForFences(dev->device, 1, &dev->fence, VK_TRUE, STRESS_FENCE_TIMEOUT_NS);
}

static double seconds_since(const struct timespec *start)
{
    struct timespec now;

    clock_gettime(CLOCK_MONOTONIC, &now);
    return (double)(now.tv_sec - start->tv_sec) + (double)(now.tv_nsec - start->tv_nsec) / 1e9;
}

/* Whether every counter of an atomic64 case holds what <draws> draws add:
 * every guard set for atomic64-exchange, one per fragment and draw on the
 * one texel, one per draw on each texel. */
static bool check_atomic64(const struct arm *arm, uint32_t draws, char *what, size_t size)
{
    const uint32_t width = atomic64_width(arm), height = atomic64_height(arm);
    const uint64_t *values = arm->pattern == PATTERN_EXCHANGE ? arm->result.map : arm->readback.map;
    uint64_t i;

    switch (arm->pattern)
    {
    case PATTERN_EXCHANGE:
        for (i = 0; i < (uint64_t)width * height; i++)
            if (values[i] != 1 && values[i] != 2)
            {
                snprintf(what, size, "guard %llu is %llu, not 1 or 2", (unsigned long long)i,
                         (unsigned long long)values[i]);
                return false;
            }
        return true;
    case PATTERN_ONE_TEXEL:
        if (values[0] != (uint64_t)draws * width * height)
        {
            snprintf(what, size, "the texel is %llu, not %llu", (unsigned long long)values[0],
                     (unsigned long long)draws * width * height);
            return false;
        }
        return true;
    case PATTERN_OWN_TEXEL:
        for (i = 0; i < (uint64_t)width * height; i++)
            if (values[i] != draws)
            {
                snprintf(what, size, "texel %llu is %llu, not %u", (unsigned long long)i,
                         (unsigned long long)values[i], draws);
                return false;
            }
        return true;
    }
    return false;
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
    if (check) vkCmdBeginQuery(cmd, arm->dev->occlusion, 0, VK_QUERY_CONTROL_PRECISE_BIT);
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
    if (check) vkCmdEndQuery(cmd, arm->dev->occlusion, 0);
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
    /* Makes the shader writes and copies the check reads visible to the
     * host. */
    VkMemoryBarrier2 to_host = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2,
                                 .srcStageMask = VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT |
                                                 VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT |
                                                 VK_PIPELINE_STAGE_2_COPY_BIT,
                                 .srcAccessMask = VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT |
                                                  VK_ACCESS_2_TRANSFER_WRITE_BIT,
                                 .dstStageMask = VK_PIPELINE_STAGE_2_HOST_BIT,
                                 .dstAccessMask = VK_ACCESS_2_HOST_READ_BIT };
    VkDependencyInfo dependency = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                    .memoryBarrierCount = 1,
                                    .pMemoryBarriers = &to_host };
    uint64_t stamps[2];

    if (check) memset(arm->result.map, 0, result_size(arm));
    vkResetQueryPool(dev->device, dev->timestamps, 0, 2);
    if (arm->kind == KIND_TIMESTAMPS)
        vkResetQueryPool(dev->device, arm->queries, 0, arm->timestamps);
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
    /* Reset by the GPU, since the slot still holds the count of the previous
     * draw arm's check. */
    if (check && (arm->kind == KIND_INDEXED || arm->kind == KIND_NONINDEXED))
        vkCmdResetQueryPool(dev->cmd, dev->occlusion, 0, 1);
    if (arm->kind == KIND_INDEXED || arm->kind == KIND_NONINDEXED)
        record_draws(arm, dev->cmd, check);
    else if (arm->kind == KIND_TIMESTAMPS)
        record_timestamps(arm, dev->cmd);
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
            uint64_t samples = 0;
            if (fragments != want)
            {
                printf("kk: FAIL %s %s: the draws produced %u fragments, not %u\n", arm->name,
                       what, fragments, want);
                return false;
            }
            if (!ok(vkGetQueryPoolResults(dev->device, dev->occlusion, 0, 1, sizeof(samples),
                                          &samples, sizeof(samples),
                                          VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WAIT_BIT),
                    "vkGetQueryPoolResults"))
                return false;
            /* The draws are right, so the arm is still timed. */
            if (samples != want)
            {
                printf("kk: FAIL %s %s: the occlusion query counted %llu samples, not %u\n",
                       arm->name, what, (unsigned long long)samples, want);
                arm->query_failed = true;
            }
        }
        else if (arm->kind == KIND_TIMESTAMPS)
        {
            char wrong[160];
            if (!check_timestamps(arm, wrong, sizeof(wrong)))
            {
                printf("kk: FAIL %s %s: %s\n", arm->name, what, wrong);
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
        if (create_device(dev, arm->robustness, false) && !setup_loads(load)) dev->failed = true;
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
    case KIND_TIMESTAMPS:
        if (!setup_timestamps(arm)) return false;
        break;
    case KIND_ATOMIC64:
        /* Not timed: stress() runs them on a device of their own. */
        return false;
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
    if (arm->counters_view) vkDestroyImageView(dev->device, arm->counters_view, NULL);
    if (arm->counters) vkDestroyImage(dev->device, arm->counters, NULL);
    if (arm->counters_memory) vkFreeMemory(dev->device, arm->counters_memory, NULL);
    if (arm->queries) vkDestroyQueryPool(dev->device, arm->queries, NULL);
    destroy_buffer(dev, &arm->vertices);
    destroy_buffer(dev, &arm->indices);
    destroy_buffer(dev, &arm->indirect);
    destroy_buffer(dev, &arm->count);
    for (i = 0; i < 4; i++) destroy_buffer(dev, &arm->sources[i]);
    for (i = 0; i < 2; i++) destroy_buffer(dev, &arm->weights[i]);
    destroy_buffer(dev, &arm->result);
    destroy_buffer(dev, &arm->readback);
    free(arm->samples);
}

static void destroy_device(struct device *dev)
{
    if (!dev->device) return;
    vkDeviceWaitIdle(dev->device);
    if (dev->descriptors) vkDestroyDescriptorPool(dev->device, dev->descriptors, NULL);
    if (dev->timestamps) vkDestroyQueryPool(dev->device, dev->timestamps, NULL);
    if (dev->occlusion) vkDestroyQueryPool(dev->device, dev->occlusion, NULL);
    if (dev->fence) vkDestroyFence(dev->device, dev->fence, NULL);
    if (dev->pool) vkDestroyCommandPool(dev->device, dev->pool, NULL);
    vkDestroyDevice(dev->device, NULL);
}

/* Runs an atomic64 case on a device of its own: draws for STRESS_SECONDS,
 * one draw per submit, then checks the counters. Prints one line and
 * returns whether the device stayed and the counters are right. A draw that
 * does not finish within STRESS_FENCE_TIMEOUT_NS ends the program with 1,
 * since the device cannot be destroyed while it runs. */
static bool stress(struct arm *arm)
{
    struct device *dev = &atomic64_device;
    struct timespec start;
    uint32_t draws = 0;
    VkResult result = VK_SUCCESS;
    char what[160], wrong[160] = "";
    bool good = false;

    memset(dev, 0, sizeof(*dev));
    arm->dev = dev;
    settings(arm, what, sizeof(what));
    if (!create_device(dev, ROBUST_OFF, true) || !setup_atomic64(arm))
    {
        printf("%s %s: FAIL, could not set up\n", arm->name, what);
        goto out;
    }
    clock_gettime(CLOCK_MONOTONIC, &start);
    while (seconds_since(&start) < STRESS_SECONDS)
    {
        if ((result = run_atomic64(arm, !draws, false)) != VK_SUCCESS) break;
        draws++;
    }
    if (result == VK_TIMEOUT)
    {
        printf("%s %s: FAIL, draw %u did not finish within %llu s\n", arm->name, what, draws + 1,
               STRESS_FENCE_TIMEOUT_NS / 1000000000ull);
        exit(1);
    }
    if (result == VK_ERROR_DEVICE_LOST)
    {
        printf("%s %s: FAIL, device lost in draw %u after %.1f s\n", arm->name, what, draws + 1,
               seconds_since(&start));
        goto out;
    }
    if (result != VK_SUCCESS)
    {
        printf("%s %s: FAIL, draw %u returned %d\n", arm->name, what, draws + 1, (int)result);
        goto out;
    }
    if (arm->pattern != PATTERN_EXCHANGE && (result = run_atomic64(arm, false, true)) != VK_SUCCESS)
    {
        printf("%s %s: FAIL, the readback returned %d\n", arm->name, what, (int)result);
        goto out;
    }
    if (!check_atomic64(arm, draws, wrong, sizeof(wrong)))
    {
        printf("%s %s: FAIL, %u draws in %.1f s, but %s\n", arm->name, what, draws,
               seconds_since(&start), wrong);
        goto out;
    }
    printf("%s %s: ok, %u draws in %.1f s without device loss\n", arm->name, what, draws,
           seconds_since(&start));
    good = true;

out:
    teardown(arm);
    destroy_device(dev);
    /* So that main's teardown passes over the arm. */
    arm->dev = NULL;
    return good;
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
    /* The atomic64 cases run only when named: they draw for STRESS_SECONDS
     * each, and a lost device can take other processes' work on the GPU
     * with it. */
    if (!any_case)
        for (i = 0; i < ARM_COUNT; i++) arms[i].selected = arms[i].kind != KIND_ATOMIC64;

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
        if (!arm->selected || arm->kind == KIND_ATOMIC64) continue;
        if (!setup(arm))
        {
            char what[160];
            settings(arm, what, sizeof(what));
            printf("%s %s: FAIL, not timed\n", arm->name, what);
            arm->failed = failed = true;
            continue;
        }
        if (arm->query_failed) failed = true;
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

    for (i = 0; i < ARM_COUNT; i++)
        if (arms[i].selected && arms[i].kind == KIND_ATOMIC64 && !stress(&arms[i]))
            failed = true;

    for (i = 0; i < ARM_COUNT; i++) teardown(&arms[i]);
    for (i = 0; i < ROBUST_COUNT; i++)
    {
        teardown(&loads[i]);
        destroy_device(&devices[i]);
    }
    vkDestroyInstance(instance, NULL);
    return failed;
}
