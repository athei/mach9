#!/bin/bash
# Writes <shader>.h, the SPIR-V of each GLSL shader here as a C word array
# named after the file (draw.vert -> draw_vert), which ../kk.c includes. Run it
# after changing a shader and commit the headers with it, so building the
# benchmark needs no shader compiler. Needs glslangValidator (Homebrew's
# glslang).
set -euo pipefail

cd "$(dirname "$0")"
for src in draw.vert draw.frag count.frag loads.comp loads_wide.comp chain.comp; do
    glslangValidator -V --target-env vulkan1.3 --vn "${src/./_}" -o "$src.h" "$src"
done
