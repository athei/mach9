#version 450
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require
#extension GL_EXT_shader_image_int64 : require

// The atomic64-one-texel and atomic64-own-texel cases: each fragment adds 1
// to texel (0, 0) of an R64_UINT storage image, or to the texel under it.
// imageAtomicAdd is always a locked atomic on KosmicKrisp.
layout(set = 0, binding = 0, r64ui) uniform u64image2D counters;

layout(push_constant) uniform Draw
{
    uint one_texel;
};

layout(location = 0) out vec4 color;

void main()
{
    ivec2 texel = one_texel != 0u ? ivec2(0) : ivec2(gl_FragCoord.xy);

    imageAtomicAdd(counters, texel, 1ul);
    color = vec4(1.0);
}
