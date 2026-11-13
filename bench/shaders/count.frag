#version 450

// The fragment shader of a draw arm's check, which counts the fragments the
// draws produce.
layout(std430, set = 0, binding = 0) buffer Counter
{
    uint fragments;
};

layout(location = 0) out vec4 color;

void main()
{
    atomicAdd(fragments, 1u);
    color = vec4(1.0);
}
