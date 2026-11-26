/*
 * vkinvariance <libvulkan.1.dylib>: check that two vertex shaders that compute
 * the same position from the same inputs give the same depth, which is what
 * a depth prepass relies on. The first pass draws a scene with one vertex
 * shader, writing depth with VK_COMPARE_OP_LESS_OR_EQUAL; the second pass
 * draws it again on the same depth attachment with another vertex shader that
 * computes the position with the same expression but has other code around
 * it, testing with VK_COMPARE_OP_EQUAL and writing no depth. A driver that
 * lets its shader compiler evaluate the position differently in the two
 * shaders, such as with fast math and no invariance, rejects fragments of the
 * second pass, and what the first pass drew stays visible there.
 *
 * Both shaders take a per-object matrix from a storage buffer, indexed by a
 * push constant, as games do. The first one (vkinvariance-prepass.vert) can
 * animate the position under a uniform that is off here; the second one
 * (vkinvariance-shade.vert) also computes a world position, a normal, a view
 * vector and the position of the previous frame for the fragment shader. The
 * scene is a bumpy mesh drawn many times with different transforms, onto a
 * 32-bit float depth attachment, so that a difference of one unit in the last
 * place shows over many pixels.
 *
 * The cases:
 *
 *   undecorated  neither shader decorates Position with Invariant, as in the
 *                games this was found with; the Vulkan specification does
 *                not require the same position here, but games rely on it
 *   depth-shift  the undecorated shaders built with DEPTH_SHIFT: both add
 *                frame.sway.y to the clip-space depth and subtract the same
 *                value as a constant, and the second one uses the sum again
 *                in another output; NIR's algebraic optimisation rewrites an
 *                inexact operation according to how many uses its result
 *                has, so it can evaluate the depth differently in the two
 *                shaders
 *   invariant    both shaders decorate Position with Invariant, so the
 *                specification requires the same position
 *   control      the shaders of the invariant case, with the second pass's
 *                viewport mapping depth to [0, CONTROL_MAX_DEPTH] instead of
 *                [0, 1], a difference far above any rounding; the case
 *                passes only when the EQUAL test leaves every covered pixel
 *                red, so a driver whose EQUAL test passes everything cannot
 *                pass the other cases
 *
 * On a KosmicKrisp that compiles vertex shaders with Metal's fast math and
 * leaves the position variant, the undecorated case leaves pixels red: the
 * animation the first shader can apply keeps the w component of its position
 * unknown to the compiler, which then fuses the multiplies and adds of the
 * same dot products differently in the two shaders.
 *
 * The first pass writes red and the second green; outside the control case, a
 * pixel the first pass covered that is still red is a failure. Prints a line
 * per case with the count of wrong pixels. Exits with 2 as soon as a case
 * cannot run, a failed setup, submit or wait or a first pass that covers too
 * little of the target included, else with 1 when a case has a wrong pixel,
 * 0 otherwise. A watchdog ends a run that hangs with 2 after a few seconds.
 * Build it against the Vulkan headers (-I) the loader was built with.
 */
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>

#include <dlfcn.h>
#include <math.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "vkinvariance-prepass.vert.h"
#include "vkinvariance-prepass-invariant.vert.h"
#include "vkinvariance-prepass-depth-shift.vert.h"
#include "vkinvariance-shade.vert.h"
#include "vkinvariance-shade-invariant.vert.h"
#include "vkinvariance-shade-depth-shift.vert.h"
#include "vkinvariance-prepass.frag.h"
#include "vkinvariance-shade.frag.h"

#define SIZE 512
#define GRID 24
#define VERTICES (GRID * GRID * 6)
#define OBJECTS 48
#define WATCHDOG_SECONDS 5
#define FENCE_TIMEOUT_NS 3000000000ull
#define CONTROL_MAX_DEPTH 0.99f
#define DEPTH_SHIFT_VALUE 3.25f /* the shaders' DEPTH_SHIFT_VALUE */

#define FUNCS(X)                                                                                   \
    X(vkEnumeratePhysicalDevices)                                                                  \
    X(vkGetPhysicalDeviceProperties)                                                               \
    X(vkGetPhysicalDeviceFeatures2)                                                                \
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
    X(vkCreateGraphicsPipelines)                                                                   \
    X(vkDestroyPipeline)                                                                           \
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
    X(vkCmdBindVertexBuffers)                                                                      \
    X(vkCmdPushConstants)                                                                          \
    X(vkCmdDraw)                                                                                   \
    X(vkCmdPipelineBarrier2)                                                                       \
    X(vkCmdBeginRendering)                                                                         \
    X(vkCmdEndRendering)                                                                           \
    X(vkCmdCopyImageToBuffer)

#define DECLARE(name) static PFN_##name name;
FUNCS(DECLARE)
#undef DECLARE

struct invariance_case
{
    const char *name;
    const uint32_t *prepass;
    size_t prepass_size;
    const uint32_t *shade;
    size_t shade_size;
    bool control;
};

#define SPIRV(code) code, sizeof(code)

static const struct invariance_case cases[] = {
    { "undecorated", SPIRV(vkinvariance_prepass_vert), SPIRV(vkinvariance_shade_vert), false },
    { "depth-shift", SPIRV(vkinvariance_prepass_depth_shift_vert),
      SPIRV(vkinvariance_shade_depth_shift_vert), false },
    { "invariant", SPIRV(vkinvariance_prepass_invariant_vert),
      SPIRV(vkinvariance_shade_invariant_vert), false },
    { "control", SPIRV(vkinvariance_prepass_invariant_vert),
      SPIRV(vkinvariance_shade_invariant_vert), true },
};

/* The layouts of the shaders' Object (std430) and Frame (std140). */
struct object
{
    float clip[4][4];
    float prev_clip[4][4];
    float world[3][4];
};

struct frame
{
    float camera[4];
    float sway[4];
};

struct vertex
{
    float position[3];
    float normal[3];
};

struct buffer
{
    VkBuffer handle;
    void *map;
};

struct image
{
    VkImage handle;
    VkImageView view;
};

typedef double mat4[4][4];

static VkInstance instance;
static VkPhysicalDevice physical;
static VkPhysicalDeviceMemoryProperties memory_properties;
static VkDevice device;
static VkQueue queue;
static uint32_t queue_family;
static VkCommandBuffer cmd;
static VkFence fence;
static VkPipelineLayout pipeline_layout;
static VkDescriptorSet descriptor_set;
static struct buffer vertices, objects, frame, readback;
static struct image colour, depth;
static uint64_t random_state = 0x9e3779b97f4a7c15ull;

static bool ok(VkResult result, const char *what)
{
    if (result == VK_SUCCESS) return true;
    printf("vkinvariance: %s failed: %d\n", what, (int)result);
    return false;
}

static void watchdog(int sig)
{
    static const char message[] = "vkinvariance: no result within the watchdog's limit\n";

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

/* A number in [lo, hi), the same sequence in every run. */
static double random_in(double lo, double hi)
{
    random_state = random_state * 6364136223846793005ull + 1442695040888963407ull;
    return lo + (hi - lo) * (double)(random_state >> 11) / (double)(1ull << 53);
}

static void multiply(mat4 out, mat4 a, mat4 b)
{
    mat4 r;
    int i, j, k;

    for (i = 0; i < 4; i++)
        for (j = 0; j < 4; j++)
        {
            r[i][j] = 0.0;
            for (k = 0; k < 4; k++) r[i][j] += a[i][k] * b[k][j];
        }
    memcpy(out, r, sizeof(r));
}

static void normalize3(double v[3])
{
    double length = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);

    v[0] /= length;
    v[1] /= length;
    v[2] /= length;
}

static void cross3(double out[3], const double a[3], const double b[3])
{
    out[0] = a[1] * b[2] - a[2] * b[1];
    out[1] = a[2] * b[0] - a[0] * b[2];
    out[2] = a[0] * b[1] - a[1] * b[0];
}

/* A left-handed perspective projection to Vulkan's clip space, depth 0 at
 * the near plane and 1 at the far plane, times a camera at eye looking at
 * target. */
static void view_projection(mat4 out, const double eye[3], const double target[3])
{
    const double near = 0.5, far = 100.0, f = 1.0 / tan(0.5 * 1.1);
    const double up[3] = { 0.0, 1.0, 0.0 };
    double forward[3], right[3], above[3];
    mat4 projection = { { f, 0.0, 0.0, 0.0 },
                        { 0.0, f, 0.0, 0.0 },
                        { 0.0, 0.0, far / (far - near), -far * near / (far - near) },
                        { 0.0, 0.0, 1.0, 0.0 } };
    mat4 view = { { 0.0 } };
    int i;

    for (i = 0; i < 3; i++) forward[i] = target[i] - eye[i];
    normalize3(forward);
    cross3(right, up, forward);
    normalize3(right);
    cross3(above, forward, right);
    for (i = 0; i < 3; i++)
    {
        view[0][i] = right[i];
        view[1][i] = above[i];
        view[2][i] = forward[i];
    }
    view[0][3] = -(right[0] * eye[0] + right[1] * eye[1] + right[2] * eye[2]);
    view[1][3] = -(above[0] * eye[0] + above[1] * eye[1] + above[2] * eye[2]);
    view[2][3] = -(forward[0] * eye[0] + forward[1] * eye[1] + forward[2] * eye[2]);
    view[3][3] = 1.0;
    multiply(out, projection, view);
}

/* Translation times a rotation about a random axis times a random scale. */
static void random_world(mat4 out, double z_min, double z_max)
{
    double axis[3] = { random_in(-1.0, 1.0), random_in(-1.0, 1.0), random_in(-1.0, 1.0) };
    double angle = random_in(0.0, 6.283), c = cos(angle), s = sin(angle), t = 1.0 - c;
    double scale[3] = { random_in(1.5, 4.5), random_in(1.0, 3.0), random_in(1.5, 4.5) };
    double z = random_in(z_min, z_max);
    double move[3] = { random_in(-0.45, 0.45) * z, random_in(-0.35, 0.35) * z, z };
    double x, y, w;
    int i, j;

    normalize3(axis);
    x = axis[0];
    y = axis[1];
    w = axis[2];
    {
        double rotation[3][3] = { { t * x * x + c, t * x * y - s * w, t * x * w + s * y },
                                  { t * x * y + s * w, t * y * y + c, t * y * w - s * x },
                                  { t * x * w - s * y, t * y * w + s * x, t * w * w + c } };

        for (i = 0; i < 3; i++)
        {
            for (j = 0; j < 3; j++) out[i][j] = rotation[i][j] * scale[j];
            out[i][3] = move[i];
        }
    }
    out[3][0] = out[3][1] = out[3][2] = 0.0;
    out[3][3] = 1.0;
}

static void store_rows(float (*out)[4], mat4 m, int rows)
{
    int i, j;

    for (i = 0; i < rows; i++)
        for (j = 0; j < 4; j++) out[i][j] = (float)m[i][j];
}

static float height(float x, float z)
{
    return 0.35f * sinf(2.9f * x + 0.7f) * cosf(2.3f * z - 0.4f);
}

static struct vertex grid_vertex(int i, int j)
{
    float x = -1.0f + 2.0f * (float)i / GRID, z = -1.0f + 2.0f * (float)j / GRID;
    float dx = 0.35f * 2.9f * cosf(2.9f * x + 0.7f) * cosf(2.3f * z - 0.4f);
    float dz = -0.35f * 2.3f * sinf(2.9f * x + 0.7f) * sinf(2.3f * z - 0.4f);
    float length = sqrtf(dx * dx + 1.0f + dz * dz);
    struct vertex v = { { x, height(x, z), z }, { -dx / length, 1.0f / length, -dz / length } };

    return v;
}

/* A bumpy grid in [-1, 1] as a triangle list. */
static void fill_vertices(struct vertex *out)
{
    static const int corners[6][2] = { { 0, 0 }, { 1, 0 }, { 0, 1 }, { 1, 0 }, { 1, 1 }, { 0, 1 } };
    int i, j, k, n = 0;

    for (j = 0; j < GRID; j++)
        for (i = 0; i < GRID; i++)
            for (k = 0; k < 6; k++) out[n++] = grid_vertex(i + corners[k][0], j + corners[k][1]);
}

static void fill_scene(struct object *out, struct frame *f)
{
    const double eye[3] = { 1.3, 2.1, -3.7 }, target[3] = { 0.2, -0.4, 20.0 };
    const double prev_eye[3] = { 1.25, 2.12, -3.82 };
    mat4 view_proj, prev_view_proj, world, m;
    int i;

    view_projection(view_proj, eye, target);
    view_projection(prev_view_proj, prev_eye, target);
    for (i = 0; i < OBJECTS; i++)
    {
        random_world(world, 6.0, 60.0);
        multiply(m, view_proj, world);
        store_rows(out[i].clip, m, 4);
        multiply(m, prev_view_proj, world);
        store_rows(out[i].prev_clip, m, 4);
        store_rows(out[i].world, world, 3);
    }
    memset(f, 0, sizeof(*f));
    f->camera[0] = (float)eye[0];
    f->camera[1] = (float)eye[1];
    f->camera[2] = (float)eye[2];
    f->sway[1] = DEPTH_SHIFT_VALUE;
    f->sway[2] = 0.08f;
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
        printf("vkinvariance: no memory type with flags %#x in %#x\n", flags, req.memoryTypeBits);
        return false;
    }
    info.memoryTypeIndex = (uint32_t)type;
    return ok(vkAllocateMemory(device, &info, NULL, out), "vkAllocateMemory");
}

/* A host-visible, mapped buffer with memory of its own. */
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
    return allocate(req, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                    &memory) &&
           ok(vkBindBufferMemory(device, out->handle, memory, 0), "vkBindBufferMemory") &&
           ok(vkMapMemory(device, memory, 0, VK_WHOLE_SIZE, 0, &out->map), "vkMapMemory");
}

static bool create_image(VkFormat format, VkImageUsageFlags usage, VkImageAspectFlags aspect,
                         struct image *out)
{
    VkImageCreateInfo info = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                               .imageType = VK_IMAGE_TYPE_2D,
                               .format = format,
                               .extent = { SIZE, SIZE, 1 },
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
                                   .subresourceRange = { aspect, 0, 1, 0, 1 } };
    VkMemoryRequirements req;
    VkDeviceMemory memory;

    if (!ok(vkCreateImage(device, &info, NULL, &out->handle), "vkCreateImage")) return false;
    vkGetImageMemoryRequirements(device, out->handle, &req);
    if (!allocate(req, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, &memory) ||
        !ok(vkBindImageMemory(device, out->handle, memory, 0), "vkBindImageMemory"))
        return false;
    view.image = out->handle;
    return ok(vkCreateImageView(device, &view, NULL, &out->view), "vkCreateImageView");
}

static bool create_device(void)
{
    VkPhysicalDeviceVulkan13Features features13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES
    };
    VkPhysicalDeviceFeatures2 features = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                                           .pNext = &features13 };
    VkPhysicalDeviceVulkan13Features enable13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        .synchronization2 = VK_TRUE,
        .dynamicRendering = VK_TRUE
    };
    float priority = 1.0f;
    VkDeviceQueueCreateInfo queue_info = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                           .queueCount = 1,
                                           .pQueuePriorities = &priority };
    VkDeviceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                                .pNext = &enable13,
                                .queueCreateInfoCount = 1,
                                .pQueueCreateInfos = &queue_info };
    VkQueueFamilyProperties families[16];
    VkPhysicalDeviceProperties props;
    uint32_t count = 1, i;
    VkResult res;

    res = vkEnumeratePhysicalDevices(instance, &count, &physical);
    if ((res != VK_SUCCESS && res != VK_INCOMPLETE) || !count)
    {
        printf("vkinvariance: no Vulkan device, result %d\n", (int)res);
        return false;
    }
    vkGetPhysicalDeviceProperties(physical, &props);
    vkGetPhysicalDeviceMemoryProperties(physical, &memory_properties);
    vkGetPhysicalDeviceFeatures2(physical, &features);
    printf("vkinvariance: device %s, driverVersion %#x\n", props.deviceName, props.driverVersion);
    if (!features13.synchronization2 || !features13.dynamicRendering)
    {
        printf("vkinvariance: the device lacks synchronization2 or dynamicRendering\n");
        return false;
    }
    count = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(physical, &count, families);
    for (i = 0; i < count; i++)
        if (families[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) break;
    if (i == count)
    {
        printf("vkinvariance: no queue family with graphics\n");
        return false;
    }
    queue_family = queue_info.queueFamilyIndex = i;
    if (!ok(vkCreateDevice(physical, &info, NULL, &device), "vkCreateDevice")) return false;
    vkGetDeviceQueue(device, queue_family, 0, &queue);
    return true;
}

static bool create_descriptors(void)
{
    VkDescriptorSetLayoutBinding bindings[2] = {
        { 0, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_VERTEX_BIT, NULL },
        { 1, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, 1, VK_SHADER_STAGE_VERTEX_BIT, NULL },
    };
    VkDescriptorSetLayoutCreateInfo set_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        .bindingCount = 2,
        .pBindings = bindings
    };
    VkPushConstantRange push = { VK_SHADER_STAGE_VERTEX_BIT, 0, 4 };
    VkPipelineLayoutCreateInfo layout_info = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                                               .setLayoutCount = 1,
                                               .pushConstantRangeCount = 1,
                                               .pPushConstantRanges = &push };
    VkDescriptorPoolSize sizes[2] = { { VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1 },
                                      { VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, 1 } };
    VkDescriptorPoolCreateInfo pool_info = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                                             .maxSets = 1,
                                             .poolSizeCount = 2,
                                             .pPoolSizes = sizes };
    VkDescriptorSetAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                                          .descriptorSetCount = 1 };
    VkDescriptorBufferInfo object_desc = { 0, 0, VK_WHOLE_SIZE };
    VkDescriptorBufferInfo frame_desc = { 0, 0, VK_WHOLE_SIZE };
    VkWriteDescriptorSet writes[2] = {
        { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
          .dstBinding = 0,
          .descriptorCount = 1,
          .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
          .pBufferInfo = &object_desc },
        { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
          .dstBinding = 1,
          .descriptorCount = 1,
          .descriptorType = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER,
          .pBufferInfo = &frame_desc },
    };
    VkDescriptorSetLayout set_layout;
    VkDescriptorPool pool;

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
    if (!ok(vkAllocateDescriptorSets(device, &alloc, &descriptor_set), "vkAllocateDescriptorSets"))
        return false;
    object_desc.buffer = objects.handle;
    frame_desc.buffer = frame.handle;
    writes[0].dstSet = writes[1].dstSet = descriptor_set;
    vkUpdateDescriptorSets(device, 2, writes, 0, NULL);
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

    if (!ok(vkCreateCommandPool(device, &pool_info, NULL, &command_pool), "vkCreateCommandPool"))
        return false;
    cmd_info.commandPool = command_pool;
    if (!ok(vkAllocateCommandBuffers(device, &cmd_info, &cmd), "vkAllocateCommandBuffers") ||
        !ok(vkCreateFence(device, &fence_info, NULL, &fence), "vkCreateFence") ||
        !create_buffer(sizeof(struct vertex) * VERTICES, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
                       &vertices) ||
        !create_buffer(sizeof(struct object) * OBJECTS, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
                       &objects) ||
        !create_buffer(sizeof(struct frame), VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT, &frame) ||
        !create_buffer(SIZE * SIZE * 4, VK_BUFFER_USAGE_TRANSFER_DST_BIT, &readback) ||
        !create_image(VK_FORMAT_R8G8B8A8_UNORM,
                      VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                      VK_IMAGE_ASPECT_COLOR_BIT, &colour) ||
        !create_image(VK_FORMAT_D32_SFLOAT, VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT,
                      VK_IMAGE_ASPECT_DEPTH_BIT, &depth))
        return false;
    fill_vertices(vertices.map);
    fill_scene(objects.map, frame.map);
    return create_descriptors();
}

static bool create_module(const uint32_t *code, size_t size, VkShaderModule *out)
{
    VkShaderModuleCreateInfo info = { .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
                                      .codeSize = size,
                                      .pCode = code };

    return ok(vkCreateShaderModule(device, &info, NULL, out), "vkCreateShaderModule");
}

/* The pipeline of the first pass (depth written, LESS_OR_EQUAL) or of the
 * second (depth kept, EQUAL), with the viewport's depth range [0, max_depth]. */
static bool create_pipeline(const uint32_t *vert, size_t vert_size, bool second, float max_depth,
                            VkPipeline *out)
{
    static const VkFormat colour_format = VK_FORMAT_R8G8B8A8_UNORM;
    VkPipelineRenderingCreateInfo rendering = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO,
        .colorAttachmentCount = 1,
        .pColorAttachmentFormats = &colour_format,
        .depthAttachmentFormat = VK_FORMAT_D32_SFLOAT
    };
    VkPipelineShaderStageCreateInfo stages[2] = {
        { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
          .stage = VK_SHADER_STAGE_VERTEX_BIT,
          .pName = "main" },
        { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
          .stage = VK_SHADER_STAGE_FRAGMENT_BIT,
          .pName = "main" },
    };
    VkVertexInputBindingDescription binding = { 0, sizeof(struct vertex),
                                                VK_VERTEX_INPUT_RATE_VERTEX };
    VkVertexInputAttributeDescription attributes[2] = {
        { 0, 0, VK_FORMAT_R32G32B32_SFLOAT, 0 },
        { 1, 0, VK_FORMAT_R32G32B32_SFLOAT, 12 },
    };
    VkPipelineVertexInputStateCreateInfo input = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
        .vertexBindingDescriptionCount = 1,
        .pVertexBindingDescriptions = &binding,
        .vertexAttributeDescriptionCount = 2,
        .pVertexAttributeDescriptions = attributes
    };
    VkPipelineInputAssemblyStateCreateInfo assembly = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
        .topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST
    };
    VkViewport viewport = { 0.0f, 0.0f, SIZE, SIZE, 0.0f, max_depth };
    VkRect2D scissor = { { 0, 0 }, { SIZE, SIZE } };
    VkPipelineViewportStateCreateInfo viewport_state = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
        .viewportCount = 1,
        .pViewports = &viewport,
        .scissorCount = 1,
        .pScissors = &scissor
    };
    VkPipelineRasterizationStateCreateInfo raster = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
        .polygonMode = VK_POLYGON_MODE_FILL,
        .cullMode = VK_CULL_MODE_NONE,
        .lineWidth = 1.0f
    };
    VkPipelineMultisampleStateCreateInfo multisample = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
        .rasterizationSamples = VK_SAMPLE_COUNT_1_BIT
    };
    VkPipelineDepthStencilStateCreateInfo depth_state = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO,
        .depthTestEnable = VK_TRUE,
        .depthWriteEnable = second ? VK_FALSE : VK_TRUE,
        .depthCompareOp = second ? VK_COMPARE_OP_EQUAL : VK_COMPARE_OP_LESS_OR_EQUAL
    };
    VkPipelineColorBlendAttachmentState blend_attachment = {
        .colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT |
                          VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT
    };
    VkPipelineColorBlendStateCreateInfo blend = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
        .attachmentCount = 1,
        .pAttachments = &blend_attachment
    };
    VkGraphicsPipelineCreateInfo info = { .sType = VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
                                          .pNext = &rendering,
                                          .stageCount = 2,
                                          .pStages = stages,
                                          .pVertexInputState = &input,
                                          .pInputAssemblyState = &assembly,
                                          .pViewportState = &viewport_state,
                                          .pRasterizationState = &raster,
                                          .pMultisampleState = &multisample,
                                          .pDepthStencilState = &depth_state,
                                          .pColorBlendState = &blend,
                                          .layout = pipeline_layout };
    bool created = false;

    if (!create_module(vert, vert_size, &stages[0].module)) return false;
    if (second ? create_module(SPIRV(vkinvariance_shade_frag), &stages[1].module)
               : create_module(SPIRV(vkinvariance_prepass_frag), &stages[1].module))
    {
        created = ok(vkCreateGraphicsPipelines(device, VK_NULL_HANDLE, 1, &info, NULL, out),
                     "vkCreateGraphicsPipelines");
        vkDestroyShaderModule(device, stages[1].module, NULL);
    }
    vkDestroyShaderModule(device, stages[0].module, NULL);
    return created;
}

static VkImageMemoryBarrier2 image_barrier(const struct image *image, VkImageAspectFlags aspect,
                                           VkPipelineStageFlags2 src_stage,
                                           VkAccessFlags2 src_access,
                                           VkPipelineStageFlags2 dst_stage,
                                           VkAccessFlags2 dst_access, VkImageLayout old_layout,
                                           VkImageLayout new_layout)
{
    VkImageMemoryBarrier2 barrier = { .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
                                      .srcStageMask = src_stage,
                                      .srcAccessMask = src_access,
                                      .dstStageMask = dst_stage,
                                      .dstAccessMask = dst_access,
                                      .oldLayout = old_layout,
                                      .newLayout = new_layout,
                                      .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                      .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                      .image = image->handle,
                                      .subresourceRange = { aspect, 0, 1, 0, 1 } };
    return barrier;
}

static void barriers(const VkImageMemoryBarrier2 *images, uint32_t count)
{
    VkDependencyInfo dep = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                             .imageMemoryBarrierCount = count,
                             .pImageMemoryBarriers = images };

    vkCmdPipelineBarrier2(cmd, &dep);
}

/* The scene, onto cleared attachments in the first pass and onto the first
 * pass's in the second. */
static void draw_pass(VkPipeline pipeline, bool second)
{
    VkRenderingAttachmentInfo colour_attachment = {
        .sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO,
        .imageView = colour.view,
        .imageLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        .loadOp = second ? VK_ATTACHMENT_LOAD_OP_LOAD : VK_ATTACHMENT_LOAD_OP_CLEAR,
        .storeOp = VK_ATTACHMENT_STORE_OP_STORE
    };
    VkRenderingAttachmentInfo depth_attachment = {
        .sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO,
        .imageView = depth.view,
        .imageLayout = VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        .loadOp = second ? VK_ATTACHMENT_LOAD_OP_LOAD : VK_ATTACHMENT_LOAD_OP_CLEAR,
        .storeOp = VK_ATTACHMENT_STORE_OP_STORE,
        .clearValue.depthStencil = { 1.0f, 0 }
    };
    VkRenderingInfo info = { .sType = VK_STRUCTURE_TYPE_RENDERING_INFO,
                             .renderArea = { { 0, 0 }, { SIZE, SIZE } },
                             .layerCount = 1,
                             .colorAttachmentCount = 1,
                             .pColorAttachments = &colour_attachment,
                             .pDepthAttachment = &depth_attachment };
    VkDeviceSize offset = 0;
    int32_t object;

    vkCmdBeginRendering(cmd, &info);
    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline);
    vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline_layout, 0, 1,
                            &descriptor_set, 0, NULL);
    vkCmdBindVertexBuffers(cmd, 0, 1, &vertices.handle, &offset);
    for (object = 0; object < OBJECTS; object++)
    {
        vkCmdPushConstants(cmd, pipeline_layout, VK_SHADER_STAGE_VERTEX_BIT, 0, sizeof(object),
                           &object);
        vkCmdDraw(cmd, VERTICES, 1, 0, 0);
    }
    vkCmdEndRendering(cmd);
}

static void record(VkPipeline first, VkPipeline second)
{
    const VkPipelineStageFlags2 depth_stages = VK_PIPELINE_STAGE_2_EARLY_FRAGMENT_TESTS_BIT |
                                               VK_PIPELINE_STAGE_2_LATE_FRAGMENT_TESTS_BIT;
    const VkAccessFlags2 depth_access = VK_ACCESS_2_DEPTH_STENCIL_ATTACHMENT_READ_BIT |
                                        VK_ACCESS_2_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
    const VkPipelineStageFlags2 colour_stage = VK_PIPELINE_STAGE_2_COLOR_ATTACHMENT_OUTPUT_BIT;
    const VkAccessFlags2 colour_access = VK_ACCESS_2_COLOR_ATTACHMENT_READ_BIT |
                                         VK_ACCESS_2_COLOR_ATTACHMENT_WRITE_BIT;
    VkImageMemoryBarrier2 before[2] = {
        image_barrier(&colour, VK_IMAGE_ASPECT_COLOR_BIT, VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
                      VK_ACCESS_2_MEMORY_READ_BIT, colour_stage, colour_access,
                      VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL),
        image_barrier(&depth, VK_IMAGE_ASPECT_DEPTH_BIT, depth_stages, depth_access, depth_stages,
                      depth_access, VK_IMAGE_LAYOUT_UNDEFINED,
                      VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL),
    };
    VkImageMemoryBarrier2 between[2] = {
        image_barrier(&colour, VK_IMAGE_ASPECT_COLOR_BIT, colour_stage, colour_access,
                      colour_stage, colour_access, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                      VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL),
        image_barrier(&depth, VK_IMAGE_ASPECT_DEPTH_BIT, depth_stages, depth_access, depth_stages,
                      depth_access, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                      VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL),
    };
    VkImageMemoryBarrier2 after = image_barrier(
        &colour, VK_IMAGE_ASPECT_COLOR_BIT, colour_stage, colour_access,
        VK_PIPELINE_STAGE_2_TRANSFER_BIT, VK_ACCESS_2_TRANSFER_READ_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL);
    VkBufferImageCopy region = { .imageSubresource = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1 },
                                 .imageExtent = { SIZE, SIZE, 1 } };
    VkBufferMemoryBarrier2 host = { .sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER_2,
                                    .srcStageMask = VK_PIPELINE_STAGE_2_TRANSFER_BIT,
                                    .srcAccessMask = VK_ACCESS_2_TRANSFER_WRITE_BIT,
                                    .dstStageMask = VK_PIPELINE_STAGE_2_HOST_BIT,
                                    .dstAccessMask = VK_ACCESS_2_HOST_READ_BIT,
                                    .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                    .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                    .buffer = readback.handle,
                                    .size = VK_WHOLE_SIZE };
    VkDependencyInfo host_dep = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                  .bufferMemoryBarrierCount = 1,
                                  .pBufferMemoryBarriers = &host };

    barriers(before, 2);
    draw_pass(first, false);
    barriers(between, 2);
    draw_pass(second, true);
    barriers(&after, 1);
    vkCmdCopyImageToBuffer(cmd, colour.handle, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                           readback.handle, 1, &region);
    vkCmdPipelineBarrier2(cmd, &host_dep);
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

/* Counts the pixels of the colour read back: red where only the first pass
 * wrote, green where the second pass wrote over it. */
static void count_pixels(uint32_t *kept, uint32_t *covered)
{
    const uint8_t *pixel = readback.map;
    uint32_t i;

    *kept = *covered = 0;
    for (i = 0; i < SIZE * SIZE; i++, pixel += 4)
    {
        if (pixel[0] == 255 && pixel[1] == 0) ++*kept;
        if (pixel[0] == 255 || pixel[1] == 255) ++*covered;
    }
}

/* Prints the result of a case from the pixels read back; returns 0 when the
 * second pass wrote every pixel the first covered, or none in the control
 * case, 1 otherwise. */
static int judge(const struct invariance_case *c, uint32_t kept, uint32_t covered)
{
    if (c->control && kept != covered)
    {
        printf("vkinvariance: %s: %u of %u covered pixels passed the second pass's EQUAL depth "
               "test at another depth\n", c->name, covered - kept, covered);
        return 1;
    }
    if (!c->control && kept)
    {
        printf("vkinvariance: %s: %u of %u covered pixels failed the second pass's EQUAL depth "
               "test\n", c->name, kept, covered);
        return 1;
    }
    printf("vkinvariance: %s: ok, %u pixels\n", c->name, covered);
    return 0;
}

/* Runs one case; returns what judge() returns, or 2 when the case could not
 * run. */
static int run_case(const struct invariance_case *c)
{
    VkCommandBufferBeginInfo begin = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                       .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VkPipeline first = VK_NULL_HANDLE, second = VK_NULL_HANDLE;
    uint32_t kept, covered;
    int status = 2;

    memset(readback.map, 0, SIZE * SIZE * 4);
    if (!create_pipeline(c->prepass, c->prepass_size, false, 1.0f, &first) ||
        !create_pipeline(c->shade, c->shade_size, true, c->control ? CONTROL_MAX_DEPTH : 1.0f,
                         &second) ||
        !ok(vkResetCommandBuffer(cmd, 0), "vkResetCommandBuffer") ||
        !ok(vkBeginCommandBuffer(cmd, &begin), "vkBeginCommandBuffer"))
        goto out;
    record(first, second);
    if (!submit()) goto out;
    count_pixels(&kept, &covered);
    if (covered < SIZE * SIZE / 4)
        printf("vkinvariance: %s: the first pass covered only %u of %u pixels\n", c->name,
               covered, SIZE * SIZE);
    else
        status = judge(c, kept, covered);

out:
    if (first) vkDestroyPipeline(device, first, NULL);
    if (second) vkDestroyPipeline(device, second, NULL);
    return status;
}

int main(int argc, char **argv)
{
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .pApplicationName = "mach9 vkinvariance",
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
        printf("vkinvariance: cannot load %s: %s\n", argv[1], dlerror());
        return 2;
    }
    get_instance_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    if (!get_instance_proc)
    {
        printf("vkinvariance: %s has no vkGetInstanceProcAddr\n", argv[1]);
        return 2;
    }
    create_instance = (PFN_vkCreateInstance)get_instance_proc(NULL, "vkCreateInstance");
    if (!create_instance || !ok(create_instance(&info, NULL, &instance), "vkCreateInstance"))
        return 2;
#define LOAD(name)                                                                                 \
    if (!(name = (PFN_##name)get_instance_proc(instance, #name)))                                  \
    {                                                                                              \
        printf("vkinvariance: the loader has no %s\n", #name);                                     \
        return 2;                                                                                  \
    }
    FUNCS(LOAD)
#undef LOAD
    if (!create_device() || !create_objects()) return 2;

    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        int result = run_case(&cases[i]);

        if (result == 2) return 2;
        if (result == 1) status = 1;
    }
    return status;
}
