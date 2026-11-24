#version 450
// The fragment shader of the second pass of smoke/vkinvariance.c: green, with
// an alpha from every input, so that the vertex shader's outputs stay in use.
// vkinvariance-shade.frag.h is its SPIR-V:
// glslangValidator -V --target-env vulkan1.3 --vn vkinvariance_shade_frag -o vkinvariance-shade.frag.h vkinvariance-shade.frag

layout(location = 0) in vec3 in_world;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec3 in_view;
layout(location = 3) in vec4 in_motion;

layout(location = 0) out vec4 colour;

void main()
{
    float light = dot(normalize(in_view), in_normal) + dot(in_motion.xy - in_motion.zw, in_world.xy);
    colour = vec4(0.0, 1.0, 0.0, fract(light));
}
