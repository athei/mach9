/* The host program bundle-wine.sh builds to learn whether wined3d can get an
 * OpenGL context on this Mac. winemac.drv only asks CGL for an accelerated
 * renderer of the main display (init_context in its opengl.c), and without
 * one its OpenGL driver does not start and wined3d finds no adapter, so the
 * smoke test changes its wined3d check then. This program makes the same
 * request. It prints the GL_RENDERER of the context it got and exits with 0,
 * or prints every renderer of the main display, with its GL_RENDERER where
 * a context can be made on it and whether it is accelerated (or that there
 * is none, or why CGL could not list them), and exits with 1. */

#define GL_SILENCE_DEPRECATION
#import <Foundation/Foundation.h>
#include <CoreGraphics/CoreGraphics.h>
#include <OpenGL/OpenGL.h>
#include <OpenGL/gl.h>
#include <stdio.h>

/* The GL_RENDERER of a context made from the pixel format <attribs> asks
 * for, or nil when there is no such pixel format or context. */
static NSString *renderer_name(const CGLPixelFormatAttribute *attribs)
{
    CGLContextObj old = CGLGetCurrentContext(), context = NULL;
    CGLPixelFormatObj pix = NULL;
    NSString *name = nil;
    const GLubyte *str;
    GLint screens;

    if (CGLChoosePixelFormat(attribs, &pix, &screens) || !pix) return nil;
    if (CGLCreateContext(pix, NULL, &context)) context = NULL;
    CGLReleasePixelFormat(pix);
    if (!context) return nil;
    if (!CGLSetCurrentContext(context) && (str = glGetString(GL_RENDERER)))
        name = [NSString stringWithUTF8String:(const char *)str];
    CGLSetCurrentContext(old);
    CGLReleaseContext(context);
    return name;
}

int main(void)
{
    @autoreleasepool
    {
        CGOpenGLDisplayMask mask = CGDisplayIDToOpenGLDisplayMask(CGMainDisplayID());
        /* winemac.drv's init_context with the legacy profile, which it asks
         * for first; its OpenGL driver does not start when that fails. */
        const CGLPixelFormatAttribute winemac[] =
        {
            kCGLPFADisplayMask, (CGLPixelFormatAttribute)mask,
            kCGLPFAAccelerated, kCGLPFAOpenGLProfile, (CGLPixelFormatAttribute)kCGLOGLPVersion_Legacy, 0,
        };
        NSMutableArray<NSString *> *names = [NSMutableArray array];
        CGLRendererInfoObj info;
        GLint count = 0, i;
        NSString *name;
        CGLError err;

        if ((name = renderer_name(winemac)))
        {
            printf("%s\n", name.UTF8String);
            return 0;
        }
        if ((err = CGLQueryRendererInfo(mask, &info, &count)))
        {
            printf("CGLQueryRendererInfo failed: %d %s\n", err, CGLErrorString(err));
            return 1;
        }
        for (i = 0; i < count; i++)
        {
            GLint id = 0, accelerated = 0;
            NSString *what;

            CGLDescribeRenderer(info, i, kCGLRPRendererID, &id);
            CGLDescribeRenderer(info, i, kCGLRPAccelerated, &accelerated);
            const CGLPixelFormatAttribute single[] =
            {
                kCGLPFADisplayMask, (CGLPixelFormatAttribute)mask,
                kCGLPFARendererID, (CGLPixelFormatAttribute)id, kCGLPFASingleRenderer, 0,
            };
            what = [NSString stringWithFormat:@"renderer 0x%08x, %s", (unsigned int)id,
                             accelerated ? "accelerated" : "not accelerated"];
            name = renderer_name(single);
            [names addObject:name ? [NSString stringWithFormat:@"%@ (%@)", name, what] : what];
        }
        CGLDestroyRendererInfo(info);
        printf("%s\n", names.count ? [names componentsJoinedByString:@"; "].UTF8String
                                   : "no OpenGL renderer for the main display");
        return 1;
    }
}
