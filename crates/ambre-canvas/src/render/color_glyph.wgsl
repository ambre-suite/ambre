// render/color_glyph.wgsl
// Full-color glyph renderer (color emoji) for the Ambre UI thread.
//
// The color atlas is Rgba8Unorm holding straight-alpha RGBA emoji bitmaps.
// Unlike glyph.wgsl (which tints an R8 coverage mask by the vertex color), the
// fragment outputs the sampled texel directly and IGNORES the vertex tint --
// an emoji carries its own colors. The vertex stage is identical to glyph.wgsl,
// so the same `GlyphVertex` layout is reused (the color attribute is unused).
//
// The pipeline blends with wgpu's ALPHA_BLENDING (src*a + dst*(1-a)), which is
// correct for the straight (non-premultiplied) alpha swash produces.

struct Uniforms {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct VertIn {
    @location(0) pos:   vec2<f32>,
    @location(1) uv:    vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VertOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@vertex
fn vs_main(v: VertIn) -> VertOut {
    var out: VertOut;
    out.clip_pos = vec4<f32>(
        v.pos.x / u.viewport.x * 2.0 - 1.0,
        1.0 - v.pos.y / u.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = v.uv;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    return textureSample(atlas, atlas_sampler, in.uv);
}
