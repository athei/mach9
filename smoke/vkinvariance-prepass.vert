#version 450
// The vertex shader of the first pass of smoke/vkinvariance.c: the object's
// clip-space matrix times the vertex, after an animation that frame.sway.x
// turns on (it is off there). With DEPTH_SHIFT, the depth gets frame.sway.y
// added and DEPTH_SHIFT_VALUE, which the probe sets it to, subtracted.
// vkinvariance-prepass.vert.h, vkinvariance-prepass-invariant.vert.h and
// vkinvariance-prepass-depth-shift.vert.h are its SPIR-V:
// glslangValidator -V --target-env vulkan1.3 --vn vkinvariance_prepass_vert -o vkinvariance-prepass.vert.h vkinvariance-prepass.vert
// glslangValidator -V --target-env vulkan1.3 -DINVARIANT --vn vkinvariance_prepass_invariant_vert -o vkinvariance-prepass-invariant.vert.h vkinvariance-prepass.vert
// glslangValidator -V --target-env vulkan1.3 -DDEPTH_SHIFT --vn vkinvariance_prepass_depth_shift_vert -o vkinvariance-prepass-depth-shift.vert.h vkinvariance-prepass.vert
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

void main()
{
    vec4 p = vec4(in_position, 1.0);

    if (frame.sway.x != 0.0)
    {
        vec3 world = vec3(dot(p, objects[pc.object].world[0]), dot(p, objects[pc.object].world[1]),
                          dot(p, objects[pc.object].world[2]));
        float phase = frame.sway.y * 1.7 + world.x * 0.31 - world.z * 0.17;
        p.xyz += vec3(sin(phase), 0.0, cos(phase * 1.3)) * frame.sway.z * p.y;
    }
    gl_Position = vec4(dot(p, objects[pc.object].clip[0]), dot(p, objects[pc.object].clip[1]),
                       dot(p, objects[pc.object].clip[2]), dot(p, objects[pc.object].clip[3]));
#ifdef DEPTH_SHIFT
    gl_Position.z = (gl_Position.z + frame.sway.y) - DEPTH_SHIFT_VALUE;
#endif
}
