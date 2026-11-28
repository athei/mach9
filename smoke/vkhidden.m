/*
 * vkhidden <libvulkan.1.dylib> [<rounds>]: present to a window that is
 * ordered out right after it was shown, and destroy the swapchain right
 * after, as the hidden part of Wine's vulkan-1 test_win32_surface does
 * through winemac, which lost the KosmicKrisp device in some runs
 * (mach9 #236).
 *
 * Each round: show a window with a layer-backed view on a CAMetalLayer, as
 * winemac's Metal view is; create a surface on the layer and a swapchain;
 * order the window out; acquire an image, submit a layout barrier for it and
 * present it, then acquire again; destroy the swapchain; and do the same
 * with a second swapchain created on the hidden window. The present blit to
 * a window that was just ordered out is still waiting for its drawable on
 * the GPU when the swapchain is destroyed. A KosmicKrisp that frees the
 * blit's resources then loses the device in about one round in five, so the
 * default of 100 rounds finds that with near certainty. A minimized window
 * does not lose it, so the program does not minimize.
 *
 * The Vulkan calls run on a thread of their own while the main thread runs
 * AppKit, as in Wine. Stops at the first lost device, since a lost device
 * resets the GPU for other processes too. Prints a line per lost device and
 * a summary line, and exits with 1 on a lost device, 2 when a step failed,
 * and 0 otherwise. Build it against the Vulkan headers (-I) the loader was
 * built with.
 */
#import <AppKit/AppKit.h>
#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>

#define VK_NO_PROTOTYPES
#define VK_USE_PLATFORM_METAL_EXT
#include <vulkan/vulkan.h>

#include <dlfcn.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>

#define INSTANCE_FUNCS(X)                                                                          \
    X(vkEnumeratePhysicalDevices)                                                                  \
    X(vkGetPhysicalDeviceQueueFamilyProperties)                                                    \
    X(vkGetPhysicalDeviceSurfaceCapabilitiesKHR)                                                   \
    X(vkCreateDevice)                                                                              \
    X(vkGetDeviceProcAddr)                                                                         \
    X(vkCreateMetalSurfaceEXT)                                                                     \
    X(vkDestroySurfaceKHR)
#define DEVICE_FUNCS(X)                                                                            \
    X(vkGetDeviceQueue)                                                                            \
    X(vkCreateCommandPool)                                                                         \
    X(vkAllocateCommandBuffers)                                                                    \
    X(vkResetCommandBuffer)                                                                        \
    X(vkBeginCommandBuffer)                                                                        \
    X(vkEndCommandBuffer)                                                                          \
    X(vkCmdPipelineBarrier)                                                                        \
    X(vkQueueSubmit)                                                                               \
    X(vkCreateFence)                                                                               \
    X(vkDestroyFence)                                                                              \
    X(vkResetFences)                                                                               \
    X(vkWaitForFences)                                                                             \
    X(vkDeviceWaitIdle)                                                                            \
    X(vkCreateSwapchainKHR)                                                                        \
    X(vkDestroySwapchainKHR)                                                                       \
    X(vkGetSwapchainImagesKHR)                                                                     \
    X(vkAcquireNextImageKHR)                                                                       \
    X(vkQueuePresentKHR)

#define DECLARE(name) static PFN_##name name;
INSTANCE_FUNCS(DECLARE)
DEVICE_FUNCS(DECLARE)
#undef DECLARE

static const char *loader_path;
static int rounds = 100;
static VkInstance instance;
static VkPhysicalDevice physical;
static uint32_t family;
static VkDevice device;
static VkQueue queue;
static VkCommandBuffer cmd;
static int presents;

/* A step that failed for another reason than a lost device ends the program. */
static void step(VkResult result, const char *what)
{
    if (result >= 0 || result == VK_ERROR_DEVICE_LOST) return;
    printf("vkhidden: %s failed: %d\n", what, (int)result);
    exit(2);
}

static void on_main(dispatch_block_t block)
{
    dispatch_sync(dispatch_get_main_queue(), block);
}

@interface MetalView : NSView
@end

@implementation MetalView
/* As winemac's WineMetalView: the layer has a device, is framebuffer only
 * and draws at the window's scale. */
- (CALayer *)makeBackingLayer
{
    CAMetalLayer *layer = [CAMetalLayer layer];
    id<MTLDevice> metal = MTLCreateSystemDefaultDevice();
    layer.device = metal;
    [metal release];
    layer.framebufferOnly = YES;
    layer.contentsScale = self.window.backingScaleFactor ? self.window.backingScaleFactor : 2.0;
    return layer;
}
@end

struct window
{
    NSWindow *window;
    MetalView *view;
    CAMetalLayer *layer;
};

static void window_show(struct window *w)
{
    on_main(^{
        w->window = [[NSWindow alloc]
            initWithContentRect:NSMakeRect(100, 100, 200, 200)
                      styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
                        backing:NSBackingStoreBuffered
                          defer:NO];
        w->window.releasedWhenClosed = NO;
        w->view = [[MetalView alloc] initWithFrame:w->window.contentView.bounds];
        w->view.wantsLayer = YES;
        [w->window.contentView addSubview:w->view];
        w->layer = [(CAMetalLayer *)w->view.layer retain];
        [w->window orderFront:nil];
    });
}

static void window_hide(struct window *w)
{
    on_main(^{
        [w->window orderOut:nil];
    });
}

static void window_close(struct window *w)
{
    on_main(^{
        [w->window close];
        [w->view removeFromSuperview];
        [w->view release];
        [w->window release];
    });
    [w->layer release];
}

static VkSwapchainKHR swapchain_create(VkSurfaceKHR surface)
{
    VkSurfaceCapabilitiesKHR caps;
    VkSwapchainKHR swapchain;

    step(vkGetPhysicalDeviceSurfaceCapabilitiesKHR(physical, surface, &caps), "vkGetPhysicalDeviceSurfaceCapabilitiesKHR");
    VkSwapchainCreateInfoKHR info = {
        .sType = VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
        .surface = surface,
        .minImageCount = caps.minImageCount,
        .imageFormat = VK_FORMAT_B8G8R8A8_UNORM,
        .imageColorSpace = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR,
        .imageExtent = caps.currentExtent,
        .imageArrayLayers = 1,
        .imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
        .imageSharingMode = VK_SHARING_MODE_EXCLUSIVE,
        .preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR,
        .compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR,
        .presentMode = VK_PRESENT_MODE_IMMEDIATE_KHR,
        .clipped = VK_TRUE,
    };
    step(vkCreateSwapchainKHR(device, &info, NULL, &swapchain), "vkCreateSwapchainKHR");
    return swapchain;
}

/* test_win32_surface_swapchain_hwnd of Wine's test: returns false on a lost
 * device. */
static bool swapchain_present(VkSwapchainKHR swapchain)
{
    VkFenceCreateInfo fence_info = {.sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    VkCommandBufferBeginInfo begin_info = {.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    VkImage images[8];
    uint32_t count = 8, index;
    VkResult result = VK_SUCCESS;
    VkFence fence;

    step(vkGetSwapchainImagesKHR(device, swapchain, &count, images), "vkGetSwapchainImagesKHR");
    step(vkCreateFence(device, &fence_info, NULL, &fence), "vkCreateFence");

    if ((result = vkAcquireNextImageKHR(device, swapchain, UINT64_MAX, VK_NULL_HANDLE, fence, &index)) < 0) goto done;
    if ((result = vkWaitForFences(device, 1, &fence, VK_FALSE, UINT64_MAX))) goto done;
    vkResetFences(device, 1, &fence);

    VkImageMemoryBarrier barrier = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
        .newLayout = VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
        .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
        .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
        .image = images[index],
        .subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1},
    };
    vkResetCommandBuffer(cmd, 0);
    vkBeginCommandBuffer(cmd, &begin_info);
    vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, 0, 0, NULL, 0,
                         NULL, 1, &barrier);
    vkEndCommandBuffer(cmd);
    VkSubmitInfo submit = {.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO, .commandBufferCount = 1, .pCommandBuffers = &cmd};
    if ((result = vkQueueSubmit(queue, 1, &submit, fence))) goto done;
    if ((result = vkWaitForFences(device, 1, &fence, VK_FALSE, UINT64_MAX))) goto done;
    vkResetFences(device, 1, &fence);

    VkPresentInfoKHR present = {
        .sType = VK_STRUCTURE_TYPE_PRESENT_INFO_KHR,
        .swapchainCount = 1,
        .pSwapchains = &swapchain,
        .pImageIndices = &index,
    };
    presents++;
    if ((result = vkQueuePresentKHR(queue, &present)) < 0) goto done;

    if ((result = vkAcquireNextImageKHR(device, swapchain, UINT64_MAX, VK_NULL_HANDLE, fence, &index)) < 0) goto done;
    result = vkWaitForFences(device, 1, &fence, VK_FALSE, UINT64_MAX);

done:
    vkDestroyFence(device, fence, NULL);
    step(result, "a swapchain step");
    return result != VK_ERROR_DEVICE_LOST;
}

/* One round: the hidden part of the test. */
static bool hidden_round(void)
{
    VkMetalSurfaceCreateInfoEXT surface_info = {.sType = VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT};
    struct window w;
    VkSwapchainKHR swapchain;
    VkSurfaceKHR surface;
    bool ok;

    window_show(&w);
    surface_info.pLayer = w.layer;
    step(vkCreateMetalSurfaceEXT(instance, &surface_info, NULL, &surface), "vkCreateMetalSurfaceEXT");

    /* created before the window is hidden, then after */
    swapchain = swapchain_create(surface);
    window_hide(&w);
    ok = swapchain_present(swapchain);
    vkDestroySwapchainKHR(device, swapchain, NULL);
    if (ok)
    {
        swapchain = swapchain_create(surface);
        ok = swapchain_present(swapchain);
        vkDestroySwapchainKHR(device, swapchain, NULL);
    }

    vkDestroySurfaceKHR(instance, surface, NULL);
    window_close(&w);
    if (!ok) printf("vkhidden: lost the device after a present to a hidden window\n");
    return ok;
}

static void *run(void *arg)
{
    VkCommandPoolCreateInfo pool_info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        .flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
        .queueFamilyIndex = family,
    };
    VkCommandBufferAllocateInfo cmd_info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        .commandBufferCount = 1,
    };
    const char *extension = "VK_KHR_swapchain";
    const float priority = 1.0f;
    VkDeviceQueueCreateInfo queue_info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
        .queueFamilyIndex = family,
        .queueCount = 1,
        .pQueuePriorities = &priority,
    };
    VkDeviceCreateInfo device_info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
        .queueCreateInfoCount = 1,
        .pQueueCreateInfos = &queue_info,
        .enabledExtensionCount = 1,
        .ppEnabledExtensionNames = &extension,
    };
    VkCommandPool pool;
    int round;
    bool ok = true;

    (void)arg;
    step(vkCreateDevice(physical, &device_info, NULL, &device), "vkCreateDevice");
#define LOAD(name) name = (PFN_##name)vkGetDeviceProcAddr(device, #name);
    DEVICE_FUNCS(LOAD)
#undef LOAD
    vkGetDeviceQueue(device, family, 0, &queue);
    step(vkCreateCommandPool(device, &pool_info, NULL, &pool), "vkCreateCommandPool");
    cmd_info.commandPool = pool;
    step(vkAllocateCommandBuffers(device, &cmd_info, &cmd), "vkAllocateCommandBuffers");

    for (round = 0; ok && round < rounds; round++) ok = hidden_round();
    /* The last round's destroys come after its last check of the device. */
    if (ok)
    {
        VkResult result = vkDeviceWaitIdle(device);
        step(result, "vkDeviceWaitIdle");
        if (result == VK_ERROR_DEVICE_LOST)
        {
            printf("vkhidden: lost the device after a present to a hidden window\n");
            ok = false;
        }
    }

    printf("vkhidden: %d rounds, %d presents, %s\n", round, presents, ok ? "no lost device" : "lost the device");
    fflush(stdout);
    exit(ok ? 0 : 1);
    return NULL;
}

int main(int argc, char **argv)
{
    const char *extensions[] = {"VK_KHR_surface", "VK_EXT_metal_surface"};
    VkApplicationInfo app = {.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO, .apiVersion = VK_API_VERSION_1_3};
    VkInstanceCreateInfo instance_info = {
        .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
        .pApplicationInfo = &app,
        .enabledExtensionCount = 2,
        .ppEnabledExtensionNames = extensions,
    };
    PFN_vkGetInstanceProcAddr get_instance_proc;
    PFN_vkCreateInstance create_instance;
    VkQueueFamilyProperties families[8];
    uint32_t count = 1;
    pthread_t thread;
    void *loader;

    if (argc < 2 || argc > 3)
    {
        printf("usage: vkhidden <libvulkan.1.dylib> [<rounds>]\n");
        return 2;
    }
    loader_path = argv[1];
    if (argc == 3) rounds = atoi(argv[2]);
    setvbuf(stdout, NULL, _IOLBF, 0);

    if (!(loader = dlopen(loader_path, RTLD_NOW | RTLD_LOCAL)))
    {
        printf("vkhidden: %s\n", dlerror());
        return 2;
    }
    get_instance_proc = (PFN_vkGetInstanceProcAddr)dlsym(loader, "vkGetInstanceProcAddr");
    create_instance = (PFN_vkCreateInstance)get_instance_proc(NULL, "vkCreateInstance");
    step(create_instance(&instance_info, NULL, &instance), "vkCreateInstance");
#define LOAD(name) name = (PFN_##name)get_instance_proc(instance, #name);
    INSTANCE_FUNCS(LOAD)
#undef LOAD
    step(vkEnumeratePhysicalDevices(instance, &count, &physical), "vkEnumeratePhysicalDevices");
    if (!count)
    {
        printf("vkhidden: no Vulkan device\n");
        return 2;
    }
    count = 8;
    vkGetPhysicalDeviceQueueFamilyProperties(physical, &count, families);
    for (family = 0; family < count && !(families[family].queueFlags & VK_QUEUE_GRAPHICS_BIT); family++)
        ;

    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    pthread_create(&thread, NULL, run, NULL);
    [NSApp run];
    return 0;
}
