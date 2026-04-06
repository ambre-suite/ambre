// render/glyph.wgsl
// Alpha-mask glyph renderer for Ambre UI Thread.
//
// The atlas is R8Unorm: each texel is a coverage value [0, 1].
// The fragment multiplies coverage by the glyph color's alpha so glyphs
// blend correctly over any background at any opacity.

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
    @location(1)       color:    vec4<f32>,
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
    out.uv    = v.uv;
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
