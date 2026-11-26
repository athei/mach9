#version 450
// The vertex shader of the second pass of smoke/vkinvariance.c: the same
// position as vkinvariance-prepass.vert without the animation, and a world
// position, a normal, a view vector and motion for the fragment shader. With
// DEPTH_SHIFT, the depth changes as in vkinvariance-prepass.vert, and the
// view vector uses the sum with frame.sway.y as well.
// vkinvariance-shade.vert.h, vkinvariance-shade-invariant.vert.h and
// vkinvariance-shade-depth-shift.vert.h are its SPIR-V:
// glslangValidator -V --target-env vulkan1.3 --vn vkinvariance_shade_vert -o vkinvariance-shade.vert.h vkinvariance-shade.vert
// glslangValidator -V --target-env vulkan1.3 -DINVARIANT --vn vkinvariance_shade_invariant_vert -o vkinvariance-shade-invariant.vert.h vkinvariance-shade.vert
// glslangValidator -V --target-env vulkan1.3 -DDEPTH_SHIFT --vn vkinvariance_shade_depth_shift_vert -o vkinvariance-shade-depth-shift.vert.h vkinvariance-shade.vert
#ifdef INVARIANT
invariant gl_Position;
#endif
#define DEPTH_SHIFT_VALUE 3.25

struct Object
{
    vec4 clip[4];
    vec4 prev_clip[4];
    vec4 world[3];
};

layout(std430, set = 0, binding = 0) readonly restrict buffer Objects
{
    Object objects[];
};

layout(std140, set = 0, binding = 1) uniform Frame
{
    vec4 camera;
    vec4 sway;
} frame;

layout(push_constant) uniform Push
{
    int object;
} pc;

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;

layout(location = 0) out vec3 out_world;
layout(location = 1) out vec3 out_normal;
layout(location = 2) out vec3 out_view;
layout(location = 3) out vec4 out_motion;

void main()
{
    vec4 p = vec4(in_position, 1.0);
    vec4 prev;

    gl_Position = vec4(dot(p, objects[pc.object].clip[0]), dot(p, objects[pc.object].clip[1]),
                       dot(p, objects[pc.object].clip[2]), dot(p, objects[pc.object].clip[3]));
#ifdef DEPTH_SHIFT
    float shifted = gl_Position.z + frame.sway.y;
    gl_Position.z = shifted - DEPTH_SHIFT_VALUE;
#endif
    out_world = vec3(dot(p, objects[pc.object].world[0]), dot(p, objects[pc.object].world[1]),
                     dot(p, objects[pc.object].world[2]));
    out_normal = normalize(vec3(dot(in_normal, objects[pc.object].world[0].xyz),
                                dot(in_normal, objects[pc.object].world[1].xyz),
                                dot(in_normal, objects[pc.object].world[2].xyz)));
#ifdef DEPTH_SHIFT
    out_view = frame.camera.xyz - out_world + vec3(shifted * 1e-9);
#else
    out_view = frame.camera.xyz - out_world;
#endif
    prev = vec4(dot(p, objects[pc.object].prev_clip[0]), dot(p, objects[pc.object].prev_clip[1]),
                dot(p, objects[pc.object].prev_clip[2]), dot(p, objects[pc.object].prev_clip[3]));
    out_motion = vec4(gl_Position.xy / gl_Position.w, prev.xy / prev.w);
}
