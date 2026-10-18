/* The host program bundle-wine.sh builds to learn whether KosmicKrisp can
 * find a GPU on this Mac. KosmicKrisp takes the first Metal device that
 * supports MTLGPUFamilyMetal4 and reports no Vulkan device without one, so
 * the smoke test skips its device checks then. This program looks through
 * the same devices the same way. It prints the name of the device
 * KosmicKrisp would take and exits with 0, or prints the name of every
 * device there is (or that there is none) and exits with 1. */

#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#include <stdio.h>

int main(void)
{
    @autoreleasepool
    {
        NSArray<id<MTLDevice>> *devices = MTLCopyAllDevices();
        NSMutableArray<NSString *> *names = [NSMutableArray array];

        for (id<MTLDevice> device in devices)
        {
            if ([device supportsFamily:MTLGPUFamilyMetal4])
            {
                printf("%s\n", device.name.UTF8String);
                return 0;
            }
            [names addObject:device.name];
        }
        printf("%s\n", names.count ? [names componentsJoinedByString:@", "].UTF8String
                                   : "no Metal device");
        return 1;
    }
}
