#version 450
// The fragment shader of the first pass of smoke/vkinvariance.c: red.
// vkinvariance-prepass.frag.h is its SPIR-V:
// glslangValidator -V --target-env vulkan1.3 --vn vkinvariance_prepass_frag -o vkinvariance-prepass.frag.h vkinvariance-prepass.frag

layout(location = 0) out vec4 colour;

void main()
{
    colour = vec4(1.0, 0.0, 0.0, 1.0);
}
