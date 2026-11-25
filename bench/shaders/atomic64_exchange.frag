#version 450
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require
#extension GL_EXT_shader_atomic_int64 : require

// The atomic64-exchange case: each fragment exchanges its own 64-bit guard
// and then the guard of its partner, the fragment next to it in the row, as
// the message passing cases of #91 do. Both are locked atomics on
// KosmicKrisp, on two neighbouring addresses.
layout(std430, set = 0, binding = 0) buffer Guards
{
    uint64_t guards[];
};

layout(push_constant) uniform Draw
{
    uint width;
};

layout(location = 0) out vec4 color;

void main()
{
    uint own = uint(gl_FragCoord.y) * width + uint(gl_FragCoord.x);
    uint64_t seen = atomicExchange(guards[own], 1ul);

    seen += atomicExchange(guards[own ^ 1u], 2ul);
    color = vec4(float(seen));
}
