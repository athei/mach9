/*
 * vkalias <libvulkan.1.dylib>: check that data written through one resource
 * reads back through another resource bound to the same range of one
 * VkDeviceMemory, in one command buffer with a pipeline barrier between the
 * write and the read, which is how D3D12 placed resources alias under
 * vkd3d-proton. A driver that makes a barrier's writes visible only to the
 * resource they went through reads stale data here.
 *
 * Only pairs whose shared contents the Vulkan specification defines are
 * tested ("Memory Aliasing" in the resources chapter):
 *
 *   buffer, buffer        both host-accessible subresources, so "data
 *                         written to one alias can be read by the other"
 *   buffer, linear image  the same, with the image in
 *                         VK_IMAGE_LAYOUT_GENERAL; the texel (x, y) is the
 *                         byte offset + y * rowPitch + x * 4 of its
 *                         vkGetImageSubresourceLayout
 *   image, image          optimal images "created with identical creation
 *                         parameters, both created with the
 *                         VK_IMAGE_CREATE_ALIAS_BIT flag set, and both bound
 *                         identically to memory"
 *
 * Each case first writes and reads the reading alias, so that a cache holds
 * its old contents, then writes every word of the range through the other
 * alias, then a barrier, then copies the reading alias to a host buffer and
 * compares every word. The write is a compute shader (vkalias.comp), or the
 * clear of a render pass for the image pair. The barrier follows the write in
 * the same compute encoder, or comes after an empty render pass (KosmicKrisp
 * then has ended the encoder that wrote), so both a barrier inside an encoder
 * and one between encoders are tested.
 *
 * On KosmicKrisp with the barriers left out, every case reads back stale
 * words except the two buffer-linear-image ones, which pass either way: they
 * show nothing about a barrier there.
 *
 * Prints a line per case and exits with 1 when a word read back is wrong in
 * any case, else 2 when a case could not run, a failed submit or wait
 * included, 0 otherwise. Build it against the Vulkan headers (-I) the loader
 * was built with.
 */
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>

#include <dlfcn.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "vkalias.comp.h"

#define WIDTH 64
#define HEIGHT 64
#define OLD_BYTE 0xa5
#define WRITE_SEED 0x5a5a0000u
#define CLEAR_VALUE 0x3c3c3c3cu

#define FUNCS(X)                                                                                   \
    X(vkDestroyInstance)                                                                           \
    X(vkEnumeratePhysicalDevices)                                                                  \
    X(vkGetPhysicalDeviceProperties)                                                               \
    X(vkGetPhysicalDeviceFeatures2)                                                                \
    X(vkGetPhysicalDeviceMemoryProperties)                                                         \
    X(vkGetPhysicalDeviceQueueFamilyProperties)                                                    \
    X(vkGetPhysicalDeviceImageFormatProperties)                                                    \
    X(vkCreateDevice)                                                                              \
    X(vkDestroyDevice)                                                                             \
    X(vkGetDeviceQueue)                                                                            \
    X(vkCreateBuffer)                                                                              \
    X(vkDestroyBuffer)                                                                             \
    X(vkGetBufferMemoryRequirements)                                                               \
    X(vkBindBufferMemory)                                                                          \
    X(vkCreateImage)                                                                               \
    X(vkDestroyImage)                                                                              \
    X(vkGetImageMemoryRequirements)                                                                \
    X(vkGetImageSubresourceLayout)                                                                 \
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
    X(vkDestroyPipeline)                                                                           \
    X(vkCreateDescriptorPool)                                                                      \
    X(vkDestroyDescriptorPool)                                                                     \
    X(vkResetDescriptorPool)                                                                       \
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
    X(vkCmdBindPipeline)                                                                           \
    X(vkCmdBindDescriptorSets)                                                                     \
    X(vkCmdPushConstants)                                                                          \
    X(vkCmdDispatch)                                                                               \
    X(vkCmdPipelineBarrier2)                                                                       \
    X(vkCmdBeginRendering)                                                                         \
    X(vkCmdEndRendering)                                                                           \
    X(vkCmdFillBuffer)                                                                             \
    X(vkCmdCopyBuffer)                                                                             \
    X(vkCmdCopyBufferToImage)                                                                      \
    X(vkCmdCopyImageToBuffer)

#define DECLARE(name) static PFN_##name name;
FUNCS(DECLARE)
#undef DECLARE

enum pair
{
    BUFFER_BUFFER,
    BUFFER_LINEAR_IMAGE,
    IMAGE_IMAGE,
};

enum boundary
{
    SAME_ENCODER,       /* the barrier right after the write */
    AFTER_RENDER_PASS,  /* an empty render pass, then the barrier */
    RENDER_PASS_WRITE,  /* the write is the clear of a render pass */
};

struct alias_case
{
    const char *name;
    enum pair pair;
    enum boundary boundary;
};

static const struct alias_case cases[] = {
    { "buffer-buffer", BUFFER_BUFFER, SAME_ENCODER },
    { "buffer-linear-image", BUFFER_LINEAR_IMAGE, SAME_ENCODER },
    { "image-image", IMAGE_IMAGE, SAME_ENCODER },
    { "buffer-buffer-after-render-pass", BUFFER_BUFFER, AFTER_RENDER_PASS },
    { "buffer-linear-image-after-render-pass", BUFFER_LINEAR_IMAGE, AFTER_RENDER_PASS },
    { "image-image-after-render-pass", IMAGE_IMAGE, AFTER_RENDER_PASS },
    { "image-image-render-pass-write", IMAGE_IMAGE, RENDER_PASS_WRITE },
};

static VkInstance instance;
static VkPhysicalDevice physical;
static VkPhysicalDeviceMemoryProperties memory_properties;
static VkDevice device;
static VkQueue queue;
static uint32_t queue_family;
static VkCommandPool command_pool;
static VkCommandBuffer cmd;
static VkFence fence;
static VkDescriptorSetLayout set_layout;
static VkPipelineLayout pipeline_layout;
static VkPipeline pipeline;
static VkDescriptorPool descriptor_pool;

struct buffer
{
    VkBuffer handle;
    VkDeviceMemory memory;
    void *map;
};

struct image
{
    VkImage handle;
    VkImageView view;
    VkDeviceMemory memory;
};

/* What every case allocates; zeroed, then released by release(). */
struct resources
{
    VkDeviceMemory shared;
    VkBuffer buffer_a, buffer_b;
    VkImage image_a, image_b;
    VkImageView view_b;
    struct buffer host, scratch, dummy_buffer;
    struct image dummy_image, target;
};

static bool ok(VkResult result, const char *what)
{
    if (result == VK_SUCCESS) return true;
    printf("vkalias: %s failed: %d\n", what, (int)result);
    return false;
}

static uint32_t written(uint32_t seed, uint32_t i)
{
    return seed ^ (i * 2654435761u);
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
        printf("vkalias: no memory type with flags %#x in %#x\n", flags, req.memoryTypeBits);
        return false;
    }
    info.memoryTypeIndex = (uint32_t)type;
    return ok(vkAllocateMemory(device, &info, NULL, out), "vkAllocateMemory");
}

static bool create_buffer(VkDeviceSize size, VkBufferUsageFlags usage, VkBuffer *out)
{
    VkBufferCreateInfo info = { .sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
                                .size = size,
                                .usage = usage,
                                .sharingMode = VK_SHARING_MODE_EXCLUSIVE };

    return ok(vkCreateBuffer(device, &info, NULL, out), "vkCreateBuffer");
}

/* A buffer with memory of its own, mapped when host visible. */
static bool create_own_buffer(VkDeviceSize size, VkBufferUsageFlags usage, bool host,
                              struct buffer *out)
{
    VkMemoryRequirements req;

    if (!create_buffer(size, usage, &out->handle)) return false;
    vkGetBufferMemoryRequirements(device, out->handle, &req);
    if (!allocate(req,
                  host ? VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT
                       : VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
                  &out->memory) ||
        !ok(vkBindBufferMemory(device, out->handle, out->memory, 0), "vkBindBufferMemory"))
        return false;
    if (host && !ok(vkMapMemory(device, out->memory, 0, VK_WHOLE_SIZE, 0, &out->map), "vkMapMemory"))
        return false;
    return true;
}

static VkImageCreateInfo image_info(VkFormat format, VkImageTiling tiling, VkImageUsageFlags usage,
                                    VkImageCreateFlags flags, uint32_t width, uint32_t height)
{
    VkImageCreateInfo info = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                               .flags = flags,
                               .imageType = VK_IMAGE_TYPE_2D,
                               .format = format,
                               .extent = { width, height, 1 },
                               .mipLevels = 1,
                               .arrayLayers = 1,
                               .samples = VK_SAMPLE_COUNT_1_BIT,
                               .tiling = tiling,
                               .usage = usage,
                               .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
                               .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    return info;
}

static bool create_view(VkImage image, VkFormat format, VkImageView *out)
{
    VkImageViewCreateInfo info = { .sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                                   .image = image,
                                   .viewType = VK_IMAGE_VIEW_TYPE_2D,
                                   .format = format,
                                   .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };

    return ok(vkCreateImageView(device, &info, NULL, out), "vkCreateImageView");
}

/* An optimal image with memory and a view of its own. */
static bool create_own_image(VkFormat format, VkImageUsageFlags usage, uint32_t size,
                             struct image *out)
{
    VkImageCreateInfo info = image_info(format, VK_IMAGE_TILING_OPTIMAL, usage, 0, size, size);
    VkMemoryRequirements req;

    if (!ok(vkCreateImage(device, &info, NULL, &out->handle), "vkCreateImage")) return false;
    vkGetImageMemoryRequirements(device, out->handle, &req);
    return allocate(req, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, &out->memory) &&
           ok(vkBindImageMemory(device, out->handle, out->memory, 0), "vkBindImageMemory") &&
           create_view(out->handle, format, &out->view);
}

static void destroy_buffer(struct buffer *b)
{
    if (b->handle) vkDestroyBuffer(device, b->handle, NULL);
    if (b->memory) vkFreeMemory(device, b->memory, NULL);
}

static void destroy_image(struct image *i)
{
    if (i->view) vkDestroyImageView(device, i->view, NULL);
    if (i->handle) vkDestroyImage(device, i->handle, NULL);
    if (i->memory) vkFreeMemory(device, i->memory, NULL);
}

static void release(struct resources *r)
{
    if (r->view_b) vkDestroyImageView(device, r->view_b, NULL);
    if (r->buffer_a) vkDestroyBuffer(device, r->buffer_a, NULL);
    if (r->buffer_b) vkDestroyBuffer(device, r->buffer_b, NULL);
    if (r->image_a) vkDestroyImage(device, r->image_a, NULL);
    if (r->image_b) vkDestroyImage(device, r->image_b, NULL);
    if (r->shared) vkFreeMemory(device, r->shared, NULL);
    destroy_buffer(&r->host);
    destroy_buffer(&r->scratch);
    destroy_buffer(&r->dummy_buffer);
    destroy_image(&r->dummy_image);
    destroy_image(&r->target);
    vkResetDescriptorPool(device, descriptor_pool, 0);
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

/* Moves images from UNDEFINED, which discards their contents, to GENERAL,
 * before anything is written. */
static void to_general(const VkImage *images, uint32_t count)
{
    VkImageMemoryBarrier2 barriers[3];
    VkDependencyInfo dep = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                             .imageMemoryBarrierCount = count,
                             .pImageMemoryBarriers = barriers };
    uint32_t i;

    for (i = 0; i < count; i++)
        barriers[i] = (VkImageMemoryBarrier2){
            .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
            .srcStageMask = VK_PIPELINE_STAGE_2_NONE,
            .dstStageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
            .dstAccessMask = VK_ACCESS_2_MEMORY_READ_BIT | VK_ACCESS_2_MEMORY_WRITE_BIT,
            .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
            .newLayout = VK_IMAGE_LAYOUT_GENERAL,
            .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .image = images[i],
            .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };
    vkCmdPipelineBarrier2(cmd, &dep);
}

/* A render pass that clears view and draws nothing. */
static void clear_pass(VkImageView view, uint32_t size, uint32_t value, VkImageLayout layout)
{
    VkRenderingAttachmentInfo color = { .sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO,
                                        .imageView = view,
                                        .imageLayout = layout,
                                        .loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR,
                                        .storeOp = VK_ATTACHMENT_STORE_OP_STORE };
    VkRenderingInfo info = { .sType = VK_STRUCTURE_TYPE_RENDERING_INFO,
                             .renderArea = { { 0, 0 }, { size, size } },
                             .layerCount = 1,
                             .colorAttachmentCount = 1,
                             .pColorAttachments = &color };

    color.clearValue.color.uint32[0] = value;
    color.clearValue.color.uint32[1] = value;
    color.clearValue.color.uint32[2] = value;
    color.clearValue.color.uint32[3] = value;
    vkCmdBeginRendering(cmd, &info);
    vkCmdEndRendering(cmd);
}

static void dispatch_write(VkBuffer buffer, VkDeviceSize buffer_size, VkImageView view,
                           uint32_t mode, uint32_t count)
{
    VkDescriptorSetAllocateInfo alloc = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                                          .descriptorPool = descriptor_pool,
                                          .descriptorSetCount = 1,
                                          .pSetLayouts = &set_layout };
    VkDescriptorBufferInfo buffer_desc = { buffer, 0, buffer_size };
    VkDescriptorImageInfo image_desc = { .imageView = view, .imageLayout = VK_IMAGE_LAYOUT_GENERAL };
    VkWriteDescriptorSet writes[2] = {
        { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
          .dstBinding = 0,
          .descriptorCount = 1,
          .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
          .pBufferInfo = &buffer_desc },
        { .sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
          .dstBinding = 1,
          .descriptorCount = 1,
          .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
          .pImageInfo = &image_desc },
    };
    uint32_t push[4] = { mode, count, WIDTH, WRITE_SEED };
    VkDescriptorSet set;

    if (!ok(vkAllocateDescriptorSets(device, &alloc, &set), "vkAllocateDescriptorSets")) abort();
    writes[0].dstSet = writes[1].dstSet = set;
    vkUpdateDescriptorSets(device, 2, writes, 0, NULL);
    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
    vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline_layout, 0, 1, &set, 0,
                            NULL);
    vkCmdPushConstants(cmd, pipeline_layout, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof(push), push);
    vkCmdDispatch(cmd, (count + 63) / 64, 1, 1);
}

/* Runs one case; returns 0 when every word read back is the written one, 1
 * when one read back is wrong, 2 when the case could not run, a failed submit
 * or wait included. */
static int run_case(const struct alias_case *c)
{
    const VkImageUsageFlags image_usage = VK_IMAGE_USAGE_STORAGE_BIT |
                                          VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                                          VK_IMAGE_USAGE_TRANSFER_DST_BIT |
                                          VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
    const VkBufferUsageFlags buffer_usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT |
                                            VK_BUFFER_USAGE_TRANSFER_SRC_BIT |
                                            VK_BUFFER_USAGE_TRANSFER_DST_BIT;
    const VkDeviceSize texels = WIDTH * HEIGHT, image_bytes = texels * 4;
    VkCommandBufferBeginInfo begin = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                       .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VkSubmitInfo submit = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                            .commandBufferCount = 1,
                            .pCommandBuffers = &cmd };
    VkBufferImageCopy region = { .imageSubresource = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1 },
                                 .imageExtent = { WIDTH, HEIGHT, 1 } };
    VkSubresourceLayout layout = { 0 };
    VkMemoryRequirements req_a, req_b, req;
    struct resources r = { 0 };
    VkDeviceSize range = 0, words;
    VkImage general[3];
    uint32_t general_count = 0, wrong = 0, first = 0, got = 0, want = 0, i;
    const uint32_t *readback;
    int status = 2;

    /* The reading alias A and the writing alias B, bound at offset 0 of one
     * allocation. */
    if (c->pair == BUFFER_BUFFER)
    {
        if (!create_buffer(image_bytes, buffer_usage, &r.buffer_a) ||
            !create_buffer(image_bytes, buffer_usage, &r.buffer_b))
            goto out;
        vkGetBufferMemoryRequirements(device, r.buffer_a, &req_a);
        vkGetBufferMemoryRequirements(device, r.buffer_b, &req_b);
        range = image_bytes;
    }
    else if (c->pair == BUFFER_LINEAR_IMAGE)
    {
        VkImageCreateInfo info = image_info(VK_FORMAT_R32_UINT, VK_IMAGE_TILING_LINEAR,
                                            VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                                                VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                                            0, WIDTH, HEIGHT);
        VkImageFormatProperties props;
        VkImageSubresource sub = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 0 };

        if (vkGetPhysicalDeviceImageFormatProperties(physical, info.format, info.imageType,
                                                     info.tiling, info.usage, info.flags,
                                                     &props) != VK_SUCCESS)
        {
            printf("vkalias: %s: no linear R32_UINT images with transfer usage\n", c->name);
            goto out;
        }
        if (!ok(vkCreateImage(device, &info, NULL, &r.image_a), "vkCreateImage")) goto out;
        vkGetImageMemoryRequirements(device, r.image_a, &req_a);
        vkGetImageSubresourceLayout(device, r.image_a, &sub, &layout);
        range = (req_a.size + 3) & ~(VkDeviceSize)3;
        if (!create_buffer(range, buffer_usage, &r.buffer_b)) goto out;
        vkGetBufferMemoryRequirements(device, r.buffer_b, &req_b);
        general[general_count++] = r.image_a;
    }
    else
    {
        VkImageCreateInfo info = image_info(VK_FORMAT_R32_UINT, VK_IMAGE_TILING_OPTIMAL,
                                            image_usage, VK_IMAGE_CREATE_ALIAS_BIT, WIDTH,
                                            HEIGHT);

        if (!ok(vkCreateImage(device, &info, NULL, &r.image_a), "vkCreateImage") ||
            !ok(vkCreateImage(device, &info, NULL, &r.image_b), "vkCreateImage"))
            goto out;
        vkGetImageMemoryRequirements(device, r.image_a, &req_a);
        vkGetImageMemoryRequirements(device, r.image_b, &req_b);
        general[general_count++] = r.image_a;
        general[general_count++] = r.image_b;
    }
    req = req_a;
    req.size = req_a.size > req_b.size ? req_a.size : req_b.size;
    req.memoryTypeBits &= req_b.memoryTypeBits;
    if (!allocate(req, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, &r.shared)) goto out;
    if (r.buffer_a && !ok(vkBindBufferMemory(device, r.buffer_a, r.shared, 0), "vkBindBufferMemory"))
        goto out;
    if (r.buffer_b && !ok(vkBindBufferMemory(device, r.buffer_b, r.shared, 0), "vkBindBufferMemory"))
        goto out;
    if (r.image_a && !ok(vkBindImageMemory(device, r.image_a, r.shared, 0), "vkBindImageMemory"))
        goto out;
    if (r.image_b)
    {
        if (!ok(vkBindImageMemory(device, r.image_b, r.shared, 0), "vkBindImageMemory") ||
            !create_view(r.image_b, VK_FORMAT_R32_UINT, &r.view_b))
            goto out;
    }

    /* The host's view of A, the old contents A gets first, and the shader's
     * unused binding. */
    if (!create_own_buffer(image_bytes, VK_BUFFER_USAGE_TRANSFER_DST_BIT, true, &r.host) ||
        !create_own_buffer(image_bytes,
                           VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT,
                           true, &r.scratch) ||
        !create_own_buffer(256, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT, false, &r.dummy_buffer) ||
        !create_own_image(VK_FORMAT_R32_UINT, VK_IMAGE_USAGE_STORAGE_BIT, 4, &r.dummy_image))
        goto out;
    if (c->boundary == AFTER_RENDER_PASS &&
        !create_own_image(VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT, 16,
                          &r.target))
        goto out;
    general[general_count++] = r.dummy_image.handle;
    memset(r.scratch.map, OLD_BYTE, image_bytes);
    memset(r.host.map, 0, image_bytes);

    if (!ok(vkResetCommandBuffer(cmd, 0), "vkResetCommandBuffer") ||
        !ok(vkBeginCommandBuffer(cmd, &begin), "vkBeginCommandBuffer"))
        goto out;
    to_general(general, general_count);
    if (r.target.handle)
    {
        VkImageMemoryBarrier2 target = {
            .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
            .srcStageMask = VK_PIPELINE_STAGE_2_NONE,
            .dstStageMask = VK_PIPELINE_STAGE_2_COLOR_ATTACHMENT_OUTPUT_BIT,
            .dstAccessMask = VK_ACCESS_2_COLOR_ATTACHMENT_WRITE_BIT,
            .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
            .newLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .image = r.target.handle,
            .subresourceRange = { VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1 } };
        VkDependencyInfo dep = { .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
                                 .imageMemoryBarrierCount = 1,
                                 .pImageMemoryBarriers = &target };
        vkCmdPipelineBarrier2(cmd, &dep);
    }

    /* Old contents through A, read once through A. */
    if (r.buffer_a)
    {
        VkBufferCopy copy = { 0, 0, image_bytes };
        vkCmdCopyBuffer(cmd, r.scratch.handle, r.buffer_a, 1, &copy);
        barrier(VK_PIPELINE_STAGE_2_TRANSFER_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
                VK_PIPELINE_STAGE_2_TRANSFER_BIT, VK_ACCESS_2_TRANSFER_READ_BIT);
        vkCmdCopyBuffer(cmd, r.buffer_a, r.scratch.handle, 1, &copy);
    }
    else
    {
        vkCmdCopyBufferToImage(cmd, r.scratch.handle, r.image_a, VK_IMAGE_LAYOUT_GENERAL, 1,
                               &region);
        barrier(VK_PIPELINE_STAGE_2_TRANSFER_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
                VK_PIPELINE_STAGE_2_TRANSFER_BIT, VK_ACCESS_2_TRANSFER_READ_BIT);
        vkCmdCopyImageToBuffer(cmd, r.image_a, VK_IMAGE_LAYOUT_GENERAL, r.scratch.handle, 1,
                               &region);
    }

    /* Every word of the range through B. */
    if (c->boundary == RENDER_PASS_WRITE)
    {
        barrier(VK_PIPELINE_STAGE_2_TRANSFER_BIT,
                VK_ACCESS_2_TRANSFER_READ_BIT | VK_ACCESS_2_TRANSFER_WRITE_BIT,
                VK_PIPELINE_STAGE_2_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_ACCESS_2_COLOR_ATTACHMENT_WRITE_BIT);
        clear_pass(r.view_b, WIDTH, CLEAR_VALUE, VK_IMAGE_LAYOUT_GENERAL);
        barrier(VK_PIPELINE_STAGE_2_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_ACCESS_2_COLOR_ATTACHMENT_WRITE_BIT, VK_PIPELINE_STAGE_2_TRANSFER_BIT,
                VK_ACCESS_2_TRANSFER_READ_BIT);
    }
    else
    {
        barrier(VK_PIPELINE_STAGE_2_TRANSFER_BIT,
                VK_ACCESS_2_TRANSFER_READ_BIT | VK_ACCESS_2_TRANSFER_WRITE_BIT,
                VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT);
        words = c->pair == IMAGE_IMAGE ? texels : range / 4;
        if (c->pair == IMAGE_IMAGE)
            dispatch_write(r.dummy_buffer.handle, 256, r.view_b, 1, (uint32_t)words);
        else
            dispatch_write(r.buffer_b, range, r.dummy_image.view, 0, (uint32_t)words);
        if (c->boundary == AFTER_RENDER_PASS)
            clear_pass(r.target.view, 16, 0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL);
        barrier(VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_STORAGE_WRITE_BIT,
                VK_PIPELINE_STAGE_2_TRANSFER_BIT, VK_ACCESS_2_TRANSFER_READ_BIT);
    }

    /* A to the host. */
    if (r.buffer_a)
    {
        VkBufferCopy copy = { 0, 0, image_bytes };
        vkCmdCopyBuffer(cmd, r.buffer_a, r.host.handle, 1, &copy);
    }
    else
        vkCmdCopyImageToBuffer(cmd, r.image_a, VK_IMAGE_LAYOUT_GENERAL, r.host.handle, 1, &region);
    barrier(VK_PIPELINE_STAGE_2_TRANSFER_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_2_HOST_BIT, VK_ACCESS_2_HOST_READ_BIT);
    if (!ok(vkEndCommandBuffer(cmd), "vkEndCommandBuffer") ||
        !ok(vkResetFences(device, 1, &fence), "vkResetFences") ||
        !ok(vkQueueSubmit(queue, 1, &submit, fence), "vkQueueSubmit") ||
        !ok(vkWaitForFences(device, 1, &fence, VK_TRUE, 10000000000ull), "vkWaitForFences"))
    {
        status = 2;
        goto out;
    }

    /* Texel (x, y) of A is word x + y * WIDTH of the host buffer. */
    readback = r.host.map;
    for (i = 0; i < texels; i++)
    {
        uint32_t x = i % WIDTH, y = i / WIDTH, expect;

        if (c->boundary == RENDER_PASS_WRITE)
            expect = CLEAR_VALUE;
        else if (c->pair == BUFFER_LINEAR_IMAGE)
            expect = written(WRITE_SEED,
                             (uint32_t)((layout.offset + y * layout.rowPitch + x * 4) / 4));
        else
            expect = written(WRITE_SEED, i);
        if (readback[i] != expect && !wrong++)
        {
            first = i;
            got = readback[i];
            want = expect;
        }
    }
    if (wrong)
    {
        printf("vkalias: %s: %u of %u words wrong, first word %u: %#x, want %#x\n", c->name,
               wrong, (unsigned)texels, first, got, want);
        status = 1;
    }
    else
    {
        printf("vkalias: %s: ok\n", c->name);
        status = 0;
    }

out:
    release(&r);
    return status;
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
        printf("vkalias: no Vulkan device, result %d\n", (int)res);
        return false;
    }
    vkGetPhysicalDeviceProperties(physical, &props);
    vkGetPhysicalDeviceMemoryProperties(physical, &memory_properties);
    vkGetPhysicalDeviceFeatures2(physical, &features);
    printf("vkalias: device %s, driverVersion %#x\n", props.deviceName, props.driverVersion);
    if (!features13.synchronization2 || !features13.dynamicRendering)
    {
        printf("vkalias: the device lacks synchronization2 or dynamicRendering\n");
        return false;
    }
    count = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(physical, &count, families);
    for (i = 0; i < count; i++)
        if ((families[i].queueFlags & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT)) ==
            (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT))
            break;
    if (i == count)
    {
        printf("vkalias: no queue family with graphics and compute\n");
        return false;
    }
    queue_family = queue_info.queueFamilyIndex = i;
    if (!ok(vkCreateDevice(physical, &info, NULL, &device), "vkCreateDevice")) return false;
    vkGetDeviceQueue(device, queue_family, 0, &queue);
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
    VkDescriptorSetLayoutBinding bindings[2] = {
        { 0, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL },
        { 1, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1, VK_SHADER_STAGE_COMPUTE_BIT, NULL },
    };
    VkDescriptorSetLayoutCreateInfo set_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        .bindingCount = 2,
        .pBindings = bindings
    };
    VkPushConstantRange push = { VK_SHADER_STAGE_COMPUTE_BIT, 0, 16 };
    VkPipelineLayoutCreateInfo layout_info = { .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                                               .setLayoutCount = 1,
                                               .pSetLayouts = &set_layout,
                                               .pushConstantRangeCount = 1,
                                               .pPushConstantRanges = &push };
    VkShaderModuleCreateInfo module_info = { .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
                                             .codeSize = sizeof(vkalias_comp),
                                             .pCode = vkalias_comp };
    VkComputePipelineCreateInfo pipeline_info = {
        .sType = VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        .stage = { .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                   .stage = VK_SHADER_STAGE_COMPUTE_BIT,
                   .pName = "main" },
    };
    VkDescriptorPoolSize sizes[2] = { { VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1 },
                                      { VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1 } };
    VkDescriptorPoolCreateInfo pool = { .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                                        .maxSets = 1,
                                        .poolSizeCount = 2,
                                        .pPoolSizes = sizes };
    VkShaderModule module;
    bool created;

    if (!ok(vkCreateCommandPool(device, &pool_info, NULL, &command_pool), "vkCreateCommandPool"))
        return false;
    cmd_info.commandPool = command_pool;
    if (!ok(vkAllocateCommandBuffers(device, &cmd_info, &cmd), "vkAllocateCommandBuffers") ||
        !ok(vkCreateFence(device, &fence_info, NULL, &fence), "vkCreateFence") ||
        !ok(vkCreateDescriptorSetLayout(device, &set_info, NULL, &set_layout),
            "vkCreateDescriptorSetLayout") ||
        !ok(vkCreatePipelineLayout(device, &layout_info, NULL, &pipeline_layout),
            "vkCreatePipelineLayout") ||
        !ok(vkCreateDescriptorPool(device, &pool, NULL, &descriptor_pool),
            "vkCreateDescriptorPool") ||
        !ok(vkCreateShaderModule(device, &module_info, NULL, &module), "vkCreateShaderModule"))
        return false;
    pipeline_info.stage.module = module;
    pipeline_info.layout = pipeline_layout;
    created = ok(vkCreateComputePipelines(device, VK_NULL_HANDLE, 1, &pipeline_info, NULL,
                                          &pipeline),
                 "vkCreateComputePipelines");
    vkDestroyShaderModule(device, module, NULL);
    return created;
}

int main(int argc, char **argv)
{
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .pApplicationName = "mach9 vkalias",
                              .apiVersion = VK_API_VERSION_1_3 };
    VkInstanceCreateInfo info = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                  .pApplicationInfo = &app };
    PFN_vkGetInstanceProcAddr get_instance_proc;
    PFN_vkCreateInstance create_instance;
    int status = 0, result;
    size_t i;
    void *loader;

    setvbuf(stdout, NULL, _IOLBF, 0);
    if (argc != 2)
    {
        printf("usage: %s <libvulkan.1.dylib>\n", argv[0]);
        return 2;
    }
    if (!(loader = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL)))
    {
        printf("vkalias: cannot load %s: %s\n", argv[1], dlerror());
        return 2;
    }
    get_instance_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    if (!get_instance_proc)
    {
        printf("vkalias: %s has no vkGetInstanceProcAddr\n", argv[1]);
        return 2;
    }
    create_instance = (PFN_vkCreateInstance)get_instance_proc(NULL, "vkCreateInstance");
    if (!create_instance || !ok(create_instance(&info, NULL, &instance), "vkCreateInstance"))
        return 2;
#define LOAD(name)                                                                                 \
    if (!(name = (PFN_##name)get_instance_proc(instance, #name)))                                  \
    {                                                                                              \
        printf("vkalias: the loader has no %s\n", #name);                                          \
        return 2;                                                                                  \
    }
    FUNCS(LOAD)
#undef LOAD
    if (!create_device() || !create_objects()) return 2;

    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
    {
        result = run_case(&cases[i]);
        if (result == 1 || (result == 2 && status == 0)) status = result;
    }
    return status;
}
